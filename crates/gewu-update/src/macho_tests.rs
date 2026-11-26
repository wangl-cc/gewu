use std::io::Cursor;

use super::*;

fn thin_header(cputype: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
    bytes.extend_from_slice(&cputype.to_le_bytes());
    bytes
}

fn fat_header(magic: u32, stride: usize, cputypes: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&magic.to_be_bytes());
    bytes.extend_from_slice(&(cputypes.len() as u32).to_be_bytes());
    for cputype in cputypes {
        let mut entry = vec![0u8; stride];
        entry[..4].copy_from_slice(&cputype.to_be_bytes());
        bytes.extend_from_slice(&entry);
    }
    bytes
}

#[test]
fn verifies_thin_and_universal_macho_architectures() {
    let arm64 = thin_header(CPU_TYPE_ARM64);
    let x86_64 = thin_header(CPU_TYPE_X86_64);
    assert!(verify_arch(Cursor::new(&arm64), "arm64").is_ok());
    assert!(verify_arch(Cursor::new(&arm64), "x86_64").is_err());
    assert!(verify_arch(Cursor::new(&x86_64), "x86_64").is_ok());
    assert!(verify_arch(Cursor::new(&x86_64), "arm64").is_err());

    let fat32 = fat_header(FAT_MAGIC, 20, &[CPU_TYPE_X86_64, CPU_TYPE_ARM64]);
    let fat64 = fat_header(FAT_MAGIC_64, 32, &[CPU_TYPE_ARM64]);
    assert!(verify_arch(Cursor::new(&fat32), "arm64").is_ok());
    assert!(verify_arch(Cursor::new(&fat32), "x86_64").is_ok());
    assert!(verify_arch(Cursor::new(&fat64), "arm64").is_ok());
    assert!(verify_arch(Cursor::new(&fat64), "x86_64").is_err());

    assert!(verify_arch(Cursor::new(b"not-macho"), "arm64").is_err());
    assert!(verify_arch(Cursor::new(&[0u8; 4]), "arm64").is_err());
    let empty_fat = fat_header(FAT_MAGIC, 20, &[]);
    assert!(verify_arch(Cursor::new(&empty_fat), "arm64").is_err());
}
