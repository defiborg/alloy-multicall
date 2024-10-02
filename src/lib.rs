pub use error::MulticallError;
pub use middleware::*;

mod contract;
mod error;
mod middleware;
/// The Multicall3 well-known information taken from <https://www.multicall3.com/deployments>.
mod multicall_address;
