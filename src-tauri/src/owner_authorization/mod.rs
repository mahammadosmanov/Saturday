pub mod binding;
pub mod canonical;
pub mod cli;
pub mod deletion;
pub mod issuer;
pub mod paths;
pub mod process;

pub const OWNER_KEY_TAG: &str = "dev.rohtolos.saturday.stage1-owner-authorization.v1";
pub const PUBLIC_KEY_SCHEMA: &str = "saturday.local-provider.stage1-owner-public-key.v1";
pub const BINDING_SCHEMA: &str = "saturday.local-provider.stage1-owner-binding.v1";
pub const ENVELOPE_SCHEMA: &str = "saturday.local-provider.stage1-owner-authorization-envelope.v1";
pub const PAYLOAD_SCHEMA: &str = "saturday.local-provider.stage1-owner-authorization-payload.v1";
pub const SIGNING_DOMAIN: &[u8] = b"SATURDAY-STAGE1-OWNER-AUTHORIZATION-V1\0";
pub const AUTHORIZATION_TTL_SECONDS: i64 = 300;
