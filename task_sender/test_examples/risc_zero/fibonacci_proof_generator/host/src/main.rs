// These constants represent the RISC-V ELF and the image ID generated by risc0-build.
// The ELF is used for proving and the ID is used for verification.
use methods::{FIBONACCI_ELF, FIBONACCI_ID};
use risc0_zkvm::{default_prover, ExecutorEnv};

const PROOF_FILE_PATH: &str = "risc_zero_fibonacci.proof";

fn main() {
    // Initialize tracing. In order to view logs, run `RUST_LOG=info cargo run`
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
        .init();

   println!("Image ID to be copied to operator/risc_zero/lib/src/lib.rs and operator/risc_zero/risc_zero_test.go: {:?}", FIBONACCI_ID);

    // An executor environment describes the configurations for the zkVM
    // including program inputs.
    // An default ExecutorEnv can be created like so:
    // `let env = ExecutorEnv::builder().build().unwrap();`
    // However, this `env` does not have any inputs.
    //
    // To add add guest input to the executor environment, use
    // ExecutorEnvBuilder::write().
    // To access this method, you'll need to use ExecutorEnv::builder(), which
    // creates an ExecutorEnvBuilder. When you're done adding input, call
    // ExecutorEnvBuilder::build().

    // For example:
    let input: u32 = 500;
    let env = ExecutorEnv::builder()
        .write(&input)
        .unwrap()
        .build()
        .unwrap();

    // Obtain the default prover.
    let prover = default_prover();

    // Produce a receipt by proving the specified ELF binary.
    let receipt = prover.prove(env, FIBONACCI_ELF).unwrap().receipt;

    // Retrieve receipt journal here.
    let vars: (u32, u32) = receipt.journal.decode().unwrap();

    let (a, b) = vars;

    println!("a: {}", a);
    println!("b: {}", b);

    let verification_result = receipt.verify(FIBONACCI_ID).is_ok();

    println!("Verification result: {}", verification_result);

    let serialized = bincode::serialize(&receipt).unwrap();

    std::fs::write(PROOF_FILE_PATH, serialized).unwrap();
}
