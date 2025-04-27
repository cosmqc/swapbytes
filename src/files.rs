use hex;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRequest(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileResponse(pub Vec<u8>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub filename: String,
    pub owner: String,
    pub description: Option<String>,
    pub hash: String,
    pub size: usize,
}

pub struct LocalFileStore {
    metadata: HashMap<String, FileMetadata>,
    files: HashMap<String, Vec<u8>>,
}

/// Each user keeps a store of the files they've uploaded.
/// The metadata is added to the DHT and shared around, the files are stored locally.
impl LocalFileStore {
    pub fn new() -> Self {
        LocalFileStore {
            metadata: HashMap::new(),
            files: HashMap::new(),
        }
    }

    /// Upload a file, pull metadata from it, and return the hash of the file
    pub fn add_file(
        &mut self,
        file_bytes: Vec<u8>,
        filename: &str,
        peer_id: &PeerId,
        description: Option<String>
    ) -> String {
        let hash = compute_hash(&file_bytes);

        let metadata = FileMetadata {
            filename: filename.to_string(),
            owner: peer_id.to_string(),
            description,
            hash: hash.clone(),
            size: file_bytes.len(),
        };

        // Add file and metadata separately (different levels of access)
        self.files.insert(hash.clone(), file_bytes);
        self.metadata.insert(hash.clone(), metadata);

        hash
    }

    pub fn get_metadata(&self, hash: &str) -> Option<&FileMetadata> {
        self.metadata.get(hash)
    }

    /// Returns a set of all the file hashes (used as an identifier)
    /// This acts as a list of the files we have, and they can request metadata from them 
    pub fn all_hashes(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    /// Get a file from local storage, wrap in Option
    pub fn get_file(&self, hash: String) -> Option<Vec<u8>> {
        self.files.get(&hash).cloned()
    }
}   

/// Generate a SHA256 hash of a given byte array (file)
pub fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
