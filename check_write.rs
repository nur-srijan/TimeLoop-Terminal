use sha2::Sha256;
use std::io::Write;

fn main() {
    let mut hasher = Sha256::new();
    let data = b"hello";
    hasher.write_all(data).unwrap();
}
