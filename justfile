default:
    @just --list

# Run all supply-chain checks: audit + deny + vet
supply-chain: audit deny vet

# Scan Cargo.lock for known vulnerabilities (RustSec advisory DB)
audit:
    cargo audit

# Check licenses, bans, sources, and advisories per deny.toml
deny:
    cargo deny check

# Verify every third-party crate has been audited per supply-chain/
vet:
    cargo vet
