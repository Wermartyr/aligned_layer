mod errors;
mod eth;

use std::fs::File;
use std::io::BufReader;
use std::io::Write;
use std::str::FromStr;
use std::{path::PathBuf, sync::Arc};

use aligned_sdk::errors::SubmitError;
use env_logger::Env;
use ethers::prelude::*;
use futures_util::StreamExt;
use log::{error, info};
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;

use aligned_sdk::models::{AlignedVerificationData, ProvingSystemId, VerificationData};

use aligned_sdk::utils::parse_proving_system;

use clap::Subcommand;
use ethers::utils::hex;
use sha3::{Digest, Keccak256};

use crate::errors::BatcherClientError;
use crate::AlignedCommands::GetVerificationKeyCommitment;
use crate::AlignedCommands::Submit;
use crate::AlignedCommands::VerifyProofOnchain;

use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct AlignedArgs {
    #[clap(subcommand)]
    pub command: AlignedCommands,
}

#[derive(Subcommand, Debug)]
pub enum AlignedCommands {
    #[clap(about = "Submit proof to the batcher")]
    Submit(SubmitArgs),
    #[clap(about = "Verify the proof was included in a verified batch on Ethereum")]
    VerifyProofOnchain(VerifyProofOnchainArgs),

    // GetVericiationKey, command name is get-vk-commitment
    #[clap(
        about = "Create verification key for proving system",
        name = "get-vk-commitment"
    )]
    GetVerificationKeyCommitment(GetVerificationKeyCommitmentArgs),
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct SubmitArgs {
    #[arg(
        name = "Batcher address",
        long = "conn",
        default_value = "ws://localhost:8080"
    )]
    connect_addr: String,
    #[arg(name = "Proving system", long = "proving_system")]
    proving_system_flag: String,
    #[arg(name = "Proof file path", long = "proof")]
    proof_file_name: PathBuf,
    #[arg(name = "Public input file name", long = "public_input")]
    pub_input_file_name: Option<PathBuf>,
    #[arg(name = "Verification key file name", long = "vk")]
    verification_key_file_name: Option<PathBuf>,
    #[arg(name = "VM prgram code file name", long = "vm_program")]
    vm_program_code_file_name: Option<PathBuf>,
    #[arg(
        name = "Number of repetitions",
        long = "repetitions",
        default_value = "1"
    )]
    repetitions: usize,
    #[arg(
        name = "Proof generator address",
        long = "proof_generator_addr",
        default_value = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    )] // defaults to anvil address 1
    proof_generator_addr: String,
    #[arg(
        name = "Aligned verification data directory Path",
        long = "aligned_verification_data_path",
        default_value = "./aligned_verification_data/"
    )]
    batch_inclusion_data_directory_path: String,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct VerifyProofOnchainArgs {
    #[arg(name = "Aligned verification data", long = "aligned-verification-data")]
    batch_inclusion_data: PathBuf,
    #[arg(
        name = "Ethereum RPC provider address",
        long = "rpc",
        default_value = "http://localhost:8545"
    )]
    eth_rpc_url: String,
    #[arg(
        name = "The Ethereum network's name",
        long = "chain",
        default_value = "devnet"
    )]
    chain: Chain,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct GetVerificationKeyCommitmentArgs {
    #[arg(name = "File name", long = "input")]
    input_file: PathBuf,
    #[arg(name = "Output file", long = "output")]
    output_file: Option<PathBuf>,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum Chain {
    Devnet,
    Holesky,
}

#[tokio::main]
async fn main() -> Result<(), aligned_sdk::errors::AlignedError> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let args: AlignedArgs = AlignedArgs::parse();

    match args.command {
        Submit(submit_args) => {
            let (ws_stream, _) = connect_async(&submit_args.connect_addr)
                .await
                .map_err(aligned_sdk::errors::SubmitError::ConnectionError)?;

            info!("WebSocket handshake has been successfully completed");
            let (ws_write, ws_read) = ws_stream.split();

            let ws_write_mutex = Arc::new(Mutex::new(ws_write));

            let batch_inclusion_data_directory_path =
                PathBuf::from(&submit_args.batch_inclusion_data_directory_path);

            std::fs::create_dir_all(&batch_inclusion_data_directory_path).map_err(|e| {
                aligned_sdk::errors::SubmitError::IoError(
                    batch_inclusion_data_directory_path.clone(),
                    e,
                )
            })?;

            let verification_data = verification_data_from_args(submit_args).unwrap();

            let verification_data_arr = vec![verification_data.clone()];

            let aligned_verification_data_vec =
                aligned_sdk::submit(ws_write_mutex, ws_read, verification_data_arr).await?;

            if let Some(aligned_verification_data_vec) = aligned_verification_data_vec {
                for aligned_verification_data in aligned_verification_data_vec {
                    save_response(
                        batch_inclusion_data_directory_path.clone(),
                        &aligned_verification_data,
                    )?;
                }
            } else {
                error!("No batch inclusion data was received from the batcher");
            }
        }
        VerifyProofOnchain(verify_inclusion_args) => {
            let contract_address = match verify_inclusion_args.chain {
                Chain::Devnet => "0x1613beB3B2C4f22Ee086B2b38C1476A3cE7f78E8",
                Chain::Holesky => "0x58F280BeBE9B34c9939C3C39e0890C81f163B623",
            };

            let batch_inclusion_file =
                File::open(verify_inclusion_args.batch_inclusion_data).unwrap();
            let reader = BufReader::new(batch_inclusion_file);
            let aligned_verification_data: AlignedVerificationData =
                serde_json::from_reader(reader).unwrap();

            // All the elements from the merkle proof have to be concatenated
            let merkle_proof: Vec<u8> = aligned_verification_data
                .batch_inclusion_proof
                .merkle_path
                .into_iter()
                .flatten()
                .collect();

            let verification_data_comm = aligned_verification_data.verification_data_commitment;

            let eth_rpc_url = verify_inclusion_args.eth_rpc_url;

            let eth_rpc_provider = Provider::<Http>::try_from(eth_rpc_url).unwrap();

            let service_manager = eth::aligned_service_manager(eth_rpc_provider, contract_address)
                .await
                .unwrap();

            let call = service_manager.verify_batch_inclusion(
                verification_data_comm.proof_commitment,
                verification_data_comm.pub_input_commitment,
                verification_data_comm.proving_system_aux_data_commitment,
                verification_data_comm.proof_generator_addr,
                aligned_verification_data.batch_merkle_root,
                merkle_proof.into(),
                aligned_verification_data.index_in_batch.into(),
            );

            match call.call().await {
                Ok(response) => {
                    if response {
                        info!("Your proof was verified in Aligned and included in the batch!");
                    } else {
                        info!("Your proof was not included in the batch.");
                    }
                }

                Err(err) => error!("Error while reading batch inclusion verification: {}", err),
            }
        }
        GetVerificationKeyCommitment(args) => {
            let content = read_file(args.input_file)?;

            let mut hasher = Keccak256::new();
            hasher.update(&content);
            let hash: [u8; 32] = hasher.finalize().into();

            info!("Commitment: {}", hex::encode(hash));
            if let Some(output_file) = args.output_file {
                let mut file = File::create(output_file.clone()).unwrap();

                file.write_all(hex::encode(hash).as_bytes())
                    .map_err(|e| BatcherClientError::IoError(output_file.clone(), e))
                    .unwrap();
            }
        }
    }

    Ok(())
}

fn verification_data_from_args(
    args: SubmitArgs,
) -> Result<VerificationData, aligned_sdk::errors::SubmitError> {
    let proving_system =
        if let Some(proving_system) = parse_proving_system(&args.proving_system_flag)? {
            proving_system
        } else {
            return Err(SubmitError::InvalidProvingSystem(args.proving_system_flag));
        };

    // Read proof file
    let proof = read_file(args.proof_file_name)?;

    let mut pub_input: Option<Vec<u8>> = None;
    let mut verification_key: Option<Vec<u8>> = None;
    let mut vm_program_code: Option<Vec<u8>> = None;

    match proving_system {
        ProvingSystemId::SP1 => {
            vm_program_code = Some(read_file_option(
                "--vm_program",
                args.vm_program_code_file_name,
            )?);
        }
        ProvingSystemId::Halo2KZG
        | ProvingSystemId::Halo2IPA
        | ProvingSystemId::GnarkPlonkBls12_381
        | ProvingSystemId::GnarkPlonkBn254
        | ProvingSystemId::Groth16Bn254 => {
            verification_key = Some(read_file_option("--vk", args.verification_key_file_name)?);
            pub_input = Some(read_file_option(
                "--public_input",
                args.pub_input_file_name,
            )?);
        }
    }

    let proof_generator_addr = Address::from_str(&args.proof_generator_addr).unwrap();

    Ok(VerificationData {
        proving_system,
        proof,
        pub_input,
        verification_key,
        vm_program_code,
        proof_generator_addr,
    })
}

fn read_file(file_name: PathBuf) -> Result<Vec<u8>, SubmitError> {
    std::fs::read(&file_name).map_err(|e| SubmitError::IoError(file_name, e))
}

fn read_file_option(param_name: &str, file_name: Option<PathBuf>) -> Result<Vec<u8>, SubmitError> {
    let file_name = file_name.ok_or(SubmitError::MissingParameter(param_name.to_string()))?;
    read_file(file_name)
}

fn save_response(
    batch_inclusion_data_directory_path: PathBuf,
    aligned_verification_data: &AlignedVerificationData,
) -> Result<(), SubmitError> {
    let batch_merkle_root = &hex::encode(aligned_verification_data.batch_merkle_root)[..8];
    let batch_inclusion_data_file_name = batch_merkle_root.to_owned()
        + "_"
        + &aligned_verification_data.index_in_batch.to_string()
        + ".json";

    let batch_inclusion_data_path =
        batch_inclusion_data_directory_path.join(&batch_inclusion_data_file_name);

    let data = serde_json::to_vec(&aligned_verification_data)?;

    let mut file = File::create(&batch_inclusion_data_path).unwrap();
    file.write_all(data.as_slice()).unwrap();
    info!(
        "Batch inclusion data written into {}",
        batch_inclusion_data_path.display()
    );

    Ok(())
}
