//! Stateless utility tools (no connection required).

use rmcp::Json;

use crate::checksums;
use crate::codec::{self, Encoding};
use crate::tools::types::{ChecksumAlgorithm, ComputeChecksumArgs, ComputeChecksumResult};

pub async fn compute_checksum(
    args: ComputeChecksumArgs,
) -> Result<Json<ComputeChecksumResult>, String> {
    let encoding: Encoding = args
        .encoding
        .parse()
        .map_err(|e: crate::codec::CodecError| format!("encoding: {e}"))?;
    let bytes = codec::decode(encoding, &args.data).map_err(|e| format!("data decode: {e}"))?;
    let (algo_name, cs) = match args.algorithm {
        ChecksumAlgorithm::Xor => ("xor", checksums::xor_checksum(&bytes)),
        ChecksumAlgorithm::Lrc => ("lrc", checksums::lrc(&bytes)),
    };
    Ok(Json(ComputeChecksumResult {
        algorithm: algo_name.to_string(),
        checksum_hex: format!("{cs:02X}"),
        checksum: cs,
        byte_count: bytes.len(),
    }))
}
