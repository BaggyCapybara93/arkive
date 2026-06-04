use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};

pub fn hash_file(path: &str) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 1024];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let result = hasher.finalize();
    let mut hex = String::with_capacity(result.len() * 2);

    for byte in result {
        hex.push_str(&format!("{:02x}", byte));
    }

    Ok(hex)
}