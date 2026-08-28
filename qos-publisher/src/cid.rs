//! Deployment id (32 raw bytes) -> CIDv0 string.
//!
//! A CIDv0 is just a base58btc-encoded sha2-256 multihash: the two prefix bytes `0x12` (sha2-256)
//! and `0x20` (32-byte digest), then the digest. Every such CID starts `Qm`.

/// Encodes a 32-byte deployment id as its CIDv0 string.
pub fn deployment_to_cid_v0(deployment: &[u8]) -> anyhow::Result<String> {
    if deployment.len() != 32 {
        anyhow::bail!(
            "deployment id must be 32 bytes, got {} — a short read here silently produces a \
             plausible-looking CID that matches no deployment",
            deployment.len()
        );
    }
    let mut multihash = Vec::with_capacity(34);
    multihash.push(0x12);
    multihash.push(0x20);
    multihash.extend_from_slice(deployment);
    Ok(bs58::encode(multihash).into_string())
}

/// Lowercase `0x` hex for a 20-byte address.
pub fn address_hex(bytes: &[u8]) -> anyhow::Result<String> {
    if bytes.len() != 20 {
        anyhow::bail!("address must be 20 bytes, got {}", bytes.len());
    }
    let mut s = String::with_capacity(42);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deployment from a real allocation record in docs/08-qos-publishing.md.
    #[test]
    fn encodes_a_known_deployment_to_its_published_cid() {
        // QmRQQTgkdtY3mxjGfbbrSJfDbBB8FAE8jSsNfPfk7SfzCN, decoded back to its digest.
        let cid = "QmRQQTgkdtY3mxjGfbbrSJfDbBB8FAE8jSsNfPfk7SfzCN";
        let mh = bs58::decode(cid).into_vec().unwrap();
        assert_eq!(mh[0], 0x12);
        assert_eq!(mh[1], 0x20);
        assert_eq!(deployment_to_cid_v0(&mh[2..]).unwrap(), cid);
    }

    #[test]
    fn every_cid_v0_starts_qm() {
        assert!(deployment_to_cid_v0(&[0u8; 32]).unwrap().starts_with("Qm"));
        assert!(deployment_to_cid_v0(&[0xff; 32]).unwrap().starts_with("Qm"));
    }

    #[test]
    fn refuses_a_wrong_length_deployment_rather_than_encoding_nonsense() {
        assert!(deployment_to_cid_v0(&[0u8; 31]).is_err());
        assert!(deployment_to_cid_v0(&[]).is_err());
    }

    #[test]
    fn addresses_are_lowercase_hex() {
        let addr = [
            0xF9, 0x2F, 0x43, 0x0d, 0xd8, 0x56, 0x7b, 0x0d, 0x46, 0x63, 0x58, 0xc7, 0x95, 0x94,
            0xab, 0x58, 0xd9, 0x19, 0xa6, 0xd4,
        ];
        assert_eq!(
            address_hex(&addr).unwrap(),
            "0xf92f430dd8567b0d466358c79594ab58d919a6d4"
        );
        assert!(address_hex(&[0u8; 19]).is_err());
    }
}
