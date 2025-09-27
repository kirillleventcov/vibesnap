use chrono;
use hex;
use sha2::{Digest, Sha256};

// Helper function to generate a unique ID
pub fn generate_id(track: &str) -> String {
    let mut hasher = Sha256::new();
    let timestamp: i64 = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    hasher.update(format!("{}{}", track, timestamp).as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..4]).to_uppercase() // Use first 4 bytes for an 8-char hex string
}
