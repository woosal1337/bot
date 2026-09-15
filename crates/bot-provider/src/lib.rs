#![forbid(unsafe_code)]

mod catalog;
mod extensions;
mod ownership;

pub use catalog::{ProviderDescriptor, ProviderProtocol, SupportLevel, discover_providers};
pub use extensions::{
    EXTENSION_CONTRACT_VERSION, ExtensionCapability, ExtensionContract, ExtensionContractError,
    extension_contract,
};
pub use ownership::{
    COMMAND_OWNERSHIP_VERSION, CommandKind, CommandOwnership, CommandOwnershipError,
};
