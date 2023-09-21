/// Encrypted Invoke transaction.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct EncryptedInvokeTransaction {
    /// Encrypted transaction data.
    pub encrypted_data: Vec<String>,

    /// Nonce for decrypting the encrypted transaction.
    pub nonce: String,

    /// t for calculating time-lock puzzle.
    pub t: u64,

    /// g for calculating time-lock puzzle.
    pub g: String,

    /// n for calculating time-lock puzzle.
    pub n: String,
}
