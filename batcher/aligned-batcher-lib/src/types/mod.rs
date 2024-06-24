use anyhow::anyhow;
use ethers::core::k256::ecdsa::SigningKey;
use ethers::signers::Signer;
use ethers::signers::Wallet;
use ethers::types::Address;
use ethers::types::Signature;
use ethers::types::SignatureError;
use lambdaworks_crypto::merkle_tree::{
    merkle::MerkleTree, proof::Proof, traits::IsMerkleTreeBackend,
};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub enum ProvingSystemId {
    GnarkPlonkBls12_381,
    GnarkPlonkBn254,
    Groth16Bn254,
    #[default]
    SP1,
    Halo2KZG,
    Halo2IPA,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VerificationData {
    pub proving_system: ProvingSystemId,
    pub proof: Vec<u8>,
    pub pub_input: Option<Vec<u8>>,
    pub verification_key: Option<Vec<u8>>,
    pub vm_program_code: Option<Vec<u8>>,
    pub proof_generator_addr: Address,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct VerificationDataCommitment {
    pub proof_commitment: [u8; 32],
    pub pub_input_commitment: [u8; 32],
    // This could be either the VM code (ELF, bytecode) or the verification key
    // depending on the proving system.
    pub proving_system_aux_data_commitment: [u8; 32],
    pub proof_generator_addr: [u8; 20],
}

impl From<VerificationData> for VerificationDataCommitment {
    fn from(verification_data: VerificationData) -> Self {
        let mut hasher = Keccak256::new();

        // compute proof commitment
        hasher.update(verification_data.proof.as_slice());
        let proof_commitment = hasher.finalize_reset().into();

        // compute public input commitment
        let mut pub_input_commitment = [0u8; 32];
        if let Some(pub_input) = &verification_data.pub_input {
            hasher.update(pub_input);
            pub_input_commitment = hasher.finalize_reset().into();
        }

        // compute proving system auxiliary data commitment
        let mut proving_system_aux_data_commitment = [0u8; 32];
        // FIXME(marian): This should probably be reworked, for the moment when the proving
        // system is SP1, `proving_system_aux_data` stands for the compiled ELF, while in the case
        // of Groth16 and PLONK, stands for the verification key.
        if let Some(vm_program_code) = &verification_data.vm_program_code {
            debug_assert_eq!(verification_data.proving_system, ProvingSystemId::SP1);
            hasher.update(vm_program_code);
            proving_system_aux_data_commitment = hasher.finalize_reset().into();
        } else if let Some(verification_key) = &verification_data.verification_key {
            hasher.update(verification_key);
            proving_system_aux_data_commitment = hasher.finalize_reset().into();
        }

        // serialize proof generator address to bytes
        let proof_generator_addr = verification_data.proof_generator_addr.into();

        VerificationDataCommitment {
            proof_commitment,
            pub_input_commitment,
            proving_system_aux_data_commitment,
            proof_generator_addr,
        }
    }
}

#[derive(Clone, Default)]
pub struct VerificationCommitmentBatch;

impl IsMerkleTreeBackend for VerificationCommitmentBatch {
    type Node = [u8; 32];
    type Data = VerificationDataCommitment;

    fn hash_data(leaf: &Self::Data) -> Self::Node {
        let mut hasher = Keccak256::new();
        hasher.update(leaf.proof_commitment);
        hasher.update(leaf.pub_input_commitment);
        hasher.update(leaf.proving_system_aux_data_commitment);
        hasher.update(leaf.proof_generator_addr);

        hasher.finalize().into()
    }

    fn hash_new_parent(child_1: &Self::Node, child_2: &Self::Node) -> Self::Node {
        let mut hasher = Keccak256::new();
        hasher.update(child_1);
        hasher.update(child_2);
        hasher.finalize().into()
    }
}

/// BatchInclusionData is the information that is retrieved to the clients once
/// the verification data sent by them has been processed by Aligned.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchInclusionData {
    pub batch_merkle_root: [u8; 32],
    pub batch_inclusion_proof: Proof<[u8; 32]>,
    pub index_in_batch: usize,
}

impl BatchInclusionData {
    pub fn new(
        verification_data_batch_index: usize,
        batch_merkle_tree: &MerkleTree<VerificationCommitmentBatch>,
    ) -> Self {
        let batch_inclusion_proof = batch_merkle_tree
            .get_proof_by_pos(verification_data_batch_index)
            .unwrap();

        BatchInclusionData {
            batch_merkle_root: batch_merkle_tree.root,
            batch_inclusion_proof,
            index_in_batch: verification_data_batch_index,
        }
    }
}

pub fn parse_proving_system(proving_system: &str) -> anyhow::Result<ProvingSystemId> {
    match proving_system {
        "GnarkPlonkBls12_381" => Ok(ProvingSystemId::GnarkPlonkBls12_381),
        "GnarkPlonkBn254" => Ok(ProvingSystemId::GnarkPlonkBn254),
        "Groth16Bn254" => Ok(ProvingSystemId::Groth16Bn254),
        "SP1" => Ok(ProvingSystemId::SP1),
        "Halo2IPA" => Ok(ProvingSystemId::Halo2IPA),
        "Halo2KZG" => Ok(ProvingSystemId::Halo2KZG),
        _ => Err(anyhow!("Invalid proving system: {}, Available proving systems are: [GnarkPlonkBls12_381, GnarkPlonkBn254, Groth16Bn254, SP1, Halo2KZG, Halo2IPA]", proving_system))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    pub verification_data: VerificationData,
    pub signature: Signature,
}

impl ClientMessage {
    /// Client message is a wrap around verification data and its signature.
    /// The signature is obtained by calculating the commitments and then hashing them.
    pub async fn new(verification_data: VerificationData, wallet: Wallet<SigningKey>) -> Self {
        let hashed_leaf = VerificationCommitmentBatch::hash_data(&verification_data.clone().into());
        let signature = wallet.sign_message(hashed_leaf).await.unwrap();

        ClientMessage {
            verification_data,
            signature,
        }
    }

    pub fn verify_signature(&self) -> Result<Address, SignatureError> {
        let hashed_leaf =
            VerificationCommitmentBatch::hash_data(&self.verification_data.clone().into());
        let recovered = self.signature.recover(hashed_leaf).unwrap();
        self.signature.verify(hashed_leaf, recovered)?;
        Ok(recovered)
    }
}
