use std::io::Read;

use crate::release::Result;

const MH_MAGIC_64: u32 = 0xfeed_facf;
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const MAX_FAT_ARCHES: u32 = 64;

fn expected_cputype(arch: &str) -> Result<u32> {
    match arch {
        "arm64" => Ok(CPU_TYPE_ARM64),
        "x86_64" => Ok(CPU_TYPE_X86_64),
        _ => Err(format!("Unsupported architecture: {arch}").into()),
    }
}

pub(crate) fn verify_arch(mut input: impl Read, expected_arch: &str) -> Result<()> {
    let target = expected_cputype(expected_arch)?;
    let mut header = [0u8; 8];
    input.read_exact(&mut header)?;
    let magic_le = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let magic_be = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);

    let matches = if magic_le == MH_MAGIC_64 {
        u32::from_le_bytes([header[4], header[5], header[6], header[7]]) == target
    } else if magic_be == MH_MAGIC_64 {
        u32::from_be_bytes([header[4], header[5], header[6], header[7]]) == target
    } else if magic_be == FAT_MAGIC || magic_be == FAT_MAGIC_64 {
        let count = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        if count == 0 || count > MAX_FAT_ARCHES {
            return Err("Invalid universal Mach-O header.".into());
        }
        let stride = if magic_be == FAT_MAGIC { 20 } else { 32 };
        let mut entries = vec![0u8; count as usize * stride];
        input.read_exact(&mut entries)?;
        entries
            .chunks_exact(stride)
            .any(|entry| u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]) == target)
    } else {
        return Err("Executable is not a 64-bit Mach-O binary.".into());
    };

    if !matches {
        return Err(
            format!("Executable does not contain the {expected_arch} architecture.").into(),
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "macho_tests.rs"]
mod tests;
