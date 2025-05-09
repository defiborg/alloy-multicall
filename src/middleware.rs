use std::{marker::PhantomData, result::Result as StdResult};

use alloy::{
    contract::{private::Network, CallBuilder, CallDecoder},
    dyn_abi::{DynSolValue, JsonAbiExt as _},
    json_abi::Function,
    primitives::{Address, Bytes, U256},
    providers::Provider,
};

use crate::{
    contract::IMulticall3::{self, IMulticall3Instance, Result as MulticallResult},
    error::MulticallError,
    multicall_address::address_by_chain_id,
};

/// Alias for [std::result::Result]<T, [MulticallError]>
pub type Result<T> = std::result::Result<T, MulticallError>;

/// An individual call within a multicall
#[derive(Debug, Clone)]
pub struct Call {
    /// The target
    target: Address,
    /// The calldata
    calldata: Bytes,
    /// Whether the call is allowed to fail
    allow_failure: bool,
    /// The decoder
    decoder: Function,
}

impl Call {
    pub fn build_call(
        target: Address,
        func: &Function,
        args: &[DynSolValue],
        allow_failure: bool,
    ) -> Self {
        Self {
            target,
            calldata: func.abi_encode_input(args).unwrap().into(),
            allow_failure,
            decoder: func.clone(),
        }
    }
}

/// The [Multicall] version - used to determine which methods of the Multicall contract to use:
/// - [`Multicall`] : `aggregate((address,bytes)[])`
/// - [`Multicall2`] : `try_aggregate(bool, (address,bytes)[])`
/// - [`Multicall3`] : `aggregate3((address,bool,bytes)[])` or
///   `aggregate3Value((address,bool,uint256,bytes)[])`
///
/// [`Multicall`]: #variant.Multicall
/// [`Multicall2`]: #variant.Multicall2
/// [`Multicall3`]: #variant.Multicall3
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum MulticallVersion {
    /// Multicall V1
    Multicall = 1,
    /// Multicall V2
    Multicall2 = 2,
    /// Multicall V3
    #[default]
    Multicall3 = 3,
}

impl From<MulticallVersion> for u8 {
    fn from(v: MulticallVersion) -> Self {
        v as u8
    }
}

impl TryFrom<u8> for MulticallVersion {
    type Error = String;
    fn try_from(v: u8) -> StdResult<Self, Self::Error> {
        match v {
            1 => Ok(MulticallVersion::Multicall),
            2 => Ok(MulticallVersion::Multicall2),
            3 => Ok(MulticallVersion::Multicall3),
            _ => Err(format!(
                "Invalid Multicall version: {v}. Accepted values: 1, 2, 3."
            )),
        }
    }
}

impl MulticallVersion {
    /// Returns true if the version is v1
    pub const fn is_v1(&self) -> bool {
        matches!(self, Self::Multicall)
    }

    /// Returns true if the version is v2     
    pub const fn is_v2(&self) -> bool {
        matches!(self, Self::Multicall2)
    }

    /// Returns true if the version is v2
    pub const fn is_v3(&self) -> bool {
        matches!(self, Self::Multicall3)
    }
}

#[derive(Debug, Clone)]
#[must_use = "Multicall does nothing unless you use `call`"]
pub struct Multicall<P, N>
where
    N: Network,
    P: Provider<N> + Clone,
{
    /// The internal calls vector
    calls: Vec<Call>,
    /// The Multicall3 contract
    contract: IMulticall3Instance<P, N>,
    /// The Multicall version to use. The default is 3.
    version: MulticallVersion,
}

impl<P, N> Multicall<P, N>
where
    N: Network,
    P: Provider<N> + Clone,
{
    /// Create a new [Multicall] instance from the given provider and known multicall address.
    ///
    /// # Note
    ///  This method doesn't verify if the address is a valid Multicall contract address.
    pub fn new(provider: P, address: Address) -> Self {
        let contract = IMulticall3::new(address, provider);
        Self {
            calls: vec![],
            contract,
            version: MulticallVersion::Multicall3,
        }
    }

    /// Create a new [Multicall] from the given provider and chain_id and chain's well-known multicall address.
    ///
    /// # Errors
    /// Returns a [`MulticallError`] if:
    /// - the provider's chain ID is [not supported](constants::MULTICALL_SUPPORTED_CHAINS) .
    pub fn with_chain_id(provider: P, chain_id: u64) -> Result<Self> {
        address_by_chain_id(chain_id)
            .ok_or(MulticallError::InvalidChainId(chain_id))
            .map(|addr| Self::new(provider, addr))
    }

    /// Create a new [Multicall] by querying the chain ID from the provider and using the chain's well-known multicall address.
    pub async fn with_provider_chain_id(provider: P) -> Result<Self> {
        let chain_id = provider
            .get_chain_id()
            .await
            .map_err(MulticallError::TransportError)?;
        Self::with_chain_id(provider, chain_id)
    }

    /// Sets the [MulticallVersion] which is used to determine which functions to use when making
    /// the contract call. The default is 3, and the default will also be used if an invalid version
    /// is provided.
    ///
    /// Version differences (adapted from [here](https://github.com/mds1/multicall#multicall---)):
    ///
    /// - Multicall (v1): This is the version of the original [MakerDAO Multicall](https://github.com/makerdao/multicall).
    ///   It provides an `aggregate` method to allow batching calls, and none of the calls are
    ///   allowed to fail.
    ///
    /// - Multicall2 (v2): The same as Multicall, but provides additional methods that allow either
    ///   all or no calls within the batch to fail. Included for backward compatibility. Use v3 to
    ///   allow failure on a per-call basis.
    ///
    /// - Multicall3 (v3): This is the recommended version, which is backwards compatible with both
    ///   Multicall & Multicall2. It provides additional methods for specifying whether calls are
    ///   allowed to fail on a per-call basis, and is also cheaper to use (so you can fit more calls
    ///   into a single request).
    ///
    /// Note: all these versions are available in the same contract address
    /// ([`constants::MULTICALL_ADDRESS`]) so changing version just changes the methods used,
    /// not the contract address.
    pub fn set_version(&mut self, version: impl TryInto<MulticallVersion>) {
        match version.try_into() {
            Ok(v) => self.version = v,
            Err(_) => self.version = MulticallVersion::Multicall3,
        }
    }

    /// Same functionality as [set_version], but uses a builder pattern to return the updated
    /// [Multicall] instance.
    ///
    /// [set_version]: #method.set_version
    pub fn with_version(&mut self, version: impl TryInto<MulticallVersion>) -> &mut Self {
        match version.try_into() {
            Ok(v) => self.version = v,
            Err(_) => self.version = MulticallVersion::Multicall3,
        };

        self
    }

    /// Appends a [`Call`] to the internal calls vector.
    ///
    /// Version specific details:
    /// - `1`: `allow_failure` is ignored.
    /// - `2`: `allow_failure` specifies whether or not this call is allowed to revert in the
    ///   multicall. If this is false for any of the calls, then the entire multicall will revert if
    ///   the individual call reverts.
    /// - `3`: `allow_failure` specifies whether or not this call is allowed to revert in the
    ///   multicall. This is on a per-call basis, however if this is `false` for an individual call
    ///   and the call reverts, then this will cause the entire multicall to revert.
    pub fn add_call(
        &mut self,
        target: Address,
        func: &Function,
        args: &[DynSolValue],
        allow_failure: bool,
    ) {
        let call = Call::build_call(target, func, args, allow_failure);

        self.calls.push(call)
    }

    /// Builder pattern to add a [Call] to the internal calls vector and return the [Multicall]. See
    /// [`add_call`] for more details.
    ///
    /// [`add_call`]: #method.add_call
    pub fn with_call(
        &mut self,
        target: Address,
        func: &Function,
        args: &[DynSolValue],
        allow_failure: bool,
    ) -> &mut Self {
        let call = Call::build_call(target, func, args, allow_failure);

        self.calls.push(call);

        self
    }

    /// Returns the current instantiated [Multicall] instance with an empty `calls` vector.
    /// This allows the user to reuse the instance to perform another aggregate query.
    pub fn clear_calls(&mut self) -> &mut Self {
        self.calls.clear();
        self
    }

    /// Queries the multicall contract via `eth_call` and returns the decoded result.
    ///
    /// Returns a vector of [StdResult]<[DynSolValue], [Bytes]> for each internal call:
    /// - Ok([DynSolValue]) if the call was successful.
    /// - Err([Bytes]) if the individual call failed whilst `allowFailure` was true, or if the
    ///   return data was empty.
    ///
    /// # Errors
    ///
    /// Returns a [MulticallError] if the Multicall call failed. This can occur due to RPC errors,
    /// or if an individual call failed whilst `allowFailure` was false.
    pub async fn call(&self) -> Result<Vec<StdResult<DynSolValue, Bytes>>> {
        match self.version {
            MulticallVersion::Multicall => {
                let call = self.as_aggregate();

                let multicall_result = call.call().await?;

                self.parse_multicall_result(multicall_result.returnData.into_iter().map(
                    |return_data| MulticallResult {
                        success: true,
                        returnData: return_data,
                    },
                ))
            }

            MulticallVersion::Multicall2 => {
                let call = self.as_try_aggregate();

                let multicall_result = call.call().await?;

                self.parse_multicall_result(multicall_result.into_iter())

            }

            MulticallVersion::Multicall3 => {
                let call = self.as_aggregate_3();

                let multicall_result = call.call().await?;

                self.parse_multicall_result(multicall_result.into_iter())

            }
        }
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the block hash
    /// of a given block number.
    ///
    /// Note: this call will return 0 if `block_number` is not one of the most recent 256 blocks.
    /// ([Reference](https://docs.soliditylang.org/en/latest/units-and-global-variables.html?highlight=blockhash#block-and-transaction-properties))
    pub fn add_get_block_hash(&mut self, block_number: impl Into<U256>) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions.get("getBlockHash").unwrap().first().unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[DynSolValue::from(block_number.into())],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the current
    /// block number.
    pub fn add_get_block_number(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions.get("getBlockNumber").unwrap().first().unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the current
    /// block coinbase address.
    pub fn add_get_current_block_coinbase(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions
            .get("getCurrentBlockCoinbase")
            .unwrap()
            .first()
            .unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the current
    /// block difficulty.
    ///
    /// Note: in a post-merge environment, the return value of this call will be the output of the
    /// randomness beacon provided by the beacon chain.
    /// ([Reference](https://eips.ethereum.org/EIPS/eip-4399#abstract))
    pub fn add_get_current_block_difficulty(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions
            .get("getCurrentBlockDifficulty")
            .unwrap()
            .first()
            .unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the current
    /// block gas limit.
    pub fn add_get_current_block_gas_limit(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions
            .get("getCurrentBlockGasLimit")
            .unwrap()
            .first()
            .unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the current
    /// block timestamp.
    pub fn add_get_current_block_timestamp(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions
            .get("getCurrentBlockTimestamp")
            .unwrap()
            .first()
            .unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the ETH
    /// balance of an address.
    pub fn add_get_eth_balance(&mut self, address: impl Into<Address>) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions.get("getEthBalance").unwrap().first().unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[DynSolValue::from(address.into())],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the last
    /// block hash.
    pub fn add_get_last_block_hash(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions.get("getLastBlockHash").unwrap().first().unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the current
    /// block base fee.
    ///
    /// Note: this call will fail if the chain that it is called on does not implement the
    /// [BASEFEE opcode](https://eips.ethereum.org/EIPS/eip-3198).
    pub fn add_get_basefee(&mut self, allow_failure: bool) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions.get("getBasefee").unwrap().first().unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            allow_failure,
        )
    }

    /// Appends a `call` to the list of calls of the Multicall instance for querying the last
    /// block hash.
    pub fn add_get_chain_id(&mut self) -> &mut Self {
        let functions = IMulticall3::abi::functions();

        let get_block_hash_function = functions.get("getChainId").unwrap().first().unwrap();

        self.with_call(
            *self.contract.address(),
            get_block_hash_function,
            &[],
            false,
        )
    }

    /// Uses the Multicall `aggregate(Call[] calldata calls)` method which returns a tuple of
    /// (uint256 blockNumber, bytes[] returnData) when called. The EVM call reverts if any
    /// individual call fails. This is used when using [MulticallVersion] V1.
    ///
    /// # Returns
    /// Returns a [CallBuilder], which uses [IMulticall3::aggregateCall] for decoding.
    pub fn as_aggregate(&self) -> CallBuilder<&P, PhantomData<IMulticall3::aggregateCall>, N> {
        let calls = self
            .calls
            .clone()
            .into_iter()
            .map(|call| IMulticall3::Call {
                target: call.target,
                callData: call.calldata,
            })
            .collect::<Vec<IMulticall3::Call>>();

        self.contract.aggregate(calls)
    }

    /// Uses the Multicall `tryAggregate(bool requireSuccess, Call[] calldata calls)` method which
    /// returns a tuple of (bool success, bytes[] returnData)[] when called. The EVM call reverts if
    /// any individual call fails. This is used when using [MulticallVersion] V2.
    ///
    /// # Returns
    /// Returns a [CallBuilder], which uses [IMulticall3::tryAggregateCall] for decoding.
    pub fn as_try_aggregate(
        &self,
    ) -> CallBuilder<&P, PhantomData<IMulticall3::tryAggregateCall>, N> {
        let mut allow_failure = true;

        let calls = self
            .calls
            .clone()
            .into_iter()
            .map(|call| {
                // If any call has `allow_failure = false`, then set allow_failure to false. The
                // `tryAggregate` contract call reverts if any of the individual calls revert.
                allow_failure &= call.allow_failure;

                IMulticall3::Call {
                    target: call.target,
                    callData: call.calldata,
                }
            })
            .collect::<Vec<IMulticall3::Call>>();

        self.contract.tryAggregate(!allow_failure, calls)
    }

    /// Uses the Multicall `aggregate3(Call3[] calldata calls)` method which returns `Results[]
    /// returnData` when called.
    ///
    /// If any of the individual calls has `allow_failure = false` then the entire multicall will
    /// fail.
    ///
    /// This is used when using [MulticallVersion] V3.
    ///
    /// # Returns
    /// Returns a [CallBuilder], which uses [IMulticall3::aggregate3Call] for decoding.
    pub fn as_aggregate_3(
        &self,
    ) -> CallBuilder<&P, PhantomData<IMulticall3::aggregate3Call>, N> {
        let calls = self
            .calls
            .clone()
            .into_iter()
            .map(|call| IMulticall3::Call3 {
                target: call.target,
                callData: call.calldata,
                allowFailure: call.allow_failure,
            })
            .collect::<Vec<IMulticall3::Call3>>();

        self.contract.aggregate3(calls)
    }

    /// Decodes the return data for each individual call result within a multicall.
    ///
    /// # Returns
    /// - Err([MulticallError]) if an individual call failed and `allow_failure` was false.
    /// - Err([MulticallError]) if there was an error decoding the return data of any individual
    ///   call.
    /// - Ok([Vec]<[StdResult]<[DynSolValue], [Bytes]>>) (see below).
    ///
    /// For each individual call it will return a [StdResult] based on the call's `success` value:
    /// - true: returns Ok([`DynSolValue`]). If there is more than 1 return value, then it will
    ///   return a [`DynSolValue`] tuple of the return values. Otherwise returns the corresponding
    ///   [`DynSolValue`] variant.
    /// - false: returns Err([Bytes]) containing the raw bytes of the error returned by the
    ///   contract.
    pub fn parse_multicall_result(
        &self,
        return_data: impl IntoIterator<Item = MulticallResult>,
    ) -> Result<Vec<StdResult<DynSolValue, Bytes>>> {
        let iter = return_data.into_iter();

        let mut results = Vec::with_capacity(self.calls.len());

        for (
            call,
            MulticallResult {
                success,
                returnData,
            },
        ) in self.calls.iter().zip(iter)
        {
            // TODO - should empty return data also be considered a call failure, and returns an
            // error when allow_failure = false?

            let result = if !success {
                if !call.allow_failure {
                    return Err(MulticallError::FailedCall);
                }

                Err(returnData)
            } else {
                let decoded = call
                    .decoder
                    .abi_decode_output(returnData)
                    .map_err(MulticallError::ContractError);

                if let Err(err) = decoded {
                    // TODO should we return out of the function here, or assign empty bytes to
                    // result? Linked to above TODO

                    return Err(err);
                } else {
                    let mut decoded = decoded.unwrap();

                    // Return the single `DynSolValue` if there's only 1 return value, otherwise
                    // return a tuple of `DynSolValue` elements
                    Ok(if decoded.len() == 1 {
                        decoded.pop().unwrap()
                    } else {
                        DynSolValue::Tuple(decoded)
                    })
                }
            };

            results.push(result);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::multicall_address::{MULTICALL_ADDRESS_DEFAULT_CHAINS, MULTICALL_DEFAULT_ADDRESS};
    use alloy::primitives::{address, utils::format_ether};
    use alloy::sol;

    sol! {
        #[derive(Debug, PartialEq)]
        #[sol(rpc, abi, extra_methods)]
        interface ERC20 {
            function totalSupply() external view returns (uint256 totalSupply);
            function balanceOf(address owner) external view returns (uint256 balance);
            function name() external view returns (string memory);
            function symbol() external view returns (string memory);
            function decimals() external view returns (uint8);
        }
    }

    #[tokio::test]
    async fn create_multicall_basic() {
        let rpc_url = "https://rpc.ankr.com/eth".parse().unwrap();
        let provider = alloy::providers::ProviderBuilder::new().on_http(rpc_url);

        // New Multicall with default address 0xcA11bde05977b3631167028862bE2a173976CA11
        let multicall =
            Multicall::with_chain_id(&provider, MULTICALL_ADDRESS_DEFAULT_CHAINS[0]).unwrap();
        assert_eq!(multicall.contract.address(), &MULTICALL_DEFAULT_ADDRESS);

        // New Multicall with user provided address
        let multicall_address = Address::ZERO;
        let multicall = Multicall::new(&provider, multicall_address);
        assert_eq!(multicall.contract.address(), &multicall_address);
    }

    #[tokio::test]
    async fn test_multicall_weth() {
        let rpc_url = "https://rpc.ankr.com/eth".parse().unwrap();
        let provider = alloy::providers::ProviderBuilder::new().on_http(rpc_url);
        let weth_address = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

        // Create the multicall instance
        let mut multicall = Multicall::with_provider_chain_id(&provider).await.unwrap();

        // Generate the WETH ERC20 instance we'll be using to create the individual calls
        let functions = ERC20::abi::functions();

        // Create the individual calls
        let name_call = functions.get("name").unwrap().first().unwrap();
        let total_supply_call = functions.get("totalSupply").unwrap().first().unwrap();
        let decimals_call = functions.get("decimals").unwrap().first().unwrap();
        let symbol_call = functions.get("symbol").unwrap().first().unwrap();

        // Add the calls
        multicall.add_call(weth_address, total_supply_call, &[], true);
        multicall.add_call(weth_address, name_call, &[], true);
        multicall.add_call(weth_address, decimals_call, &[], true);
        multicall.add_call(weth_address, symbol_call, &[], true);

        // Add the same calls via the builder pattern
        multicall
            .with_call(weth_address, total_supply_call, &[], true)
            .with_call(weth_address, name_call, &[], true)
            .with_call(weth_address, decimals_call, &[], true)
            .with_call(weth_address, symbol_call, &[], true)
            .add_get_chain_id();

        // Send and await the multicall results

        // MulticallV1
        multicall.set_version(1);
        let results = multicall.call().await.unwrap();
        assert_results(results);

        // MulticallV2
        multicall.set_version(2);
        let results = multicall.call().await.unwrap();
        assert_results(results);

        // MulticallV3
        multicall.set_version(3);
        let results = multicall.call().await.unwrap();
        assert_results(results);
    }

    #[tokio::test]
    async fn multicall_specific_methods() {
        let rpc_url = "https://rpc.ankr.com/eth".parse().unwrap();
        let provider = alloy::providers::ProviderBuilder::new().on_http(rpc_url);
        let mut multicall = Multicall::new(provider, MULTICALL_DEFAULT_ADDRESS);

        multicall
            .add_get_basefee(false)
            .add_get_block_hash(U256::from(20669406))
            .add_get_block_number()
            .add_get_chain_id()
            .add_get_current_block_coinbase()
            .add_get_current_block_difficulty()
            .add_get_current_block_gas_limit()
            .add_get_current_block_timestamp()
            .add_get_last_block_hash()
            .add_get_eth_balance(address!("3bfc20f0b9afcace800d73d2191166ff16540258"));

        let results = multicall.call().await.unwrap();

        let chain_id = results
            .get(3)
            .unwrap()
            .as_ref()
            .unwrap()
            .as_uint()
            .unwrap()
            .0
            .to::<u64>();
        let gas_limit = results
            .get(6)
            .unwrap()
            .as_ref()
            .unwrap()
            .as_uint()
            .unwrap()
            .0
            .to::<u64>();
        let eth_balance = format_ether(
            results
                .get(9)
                .unwrap()
                .as_ref()
                .unwrap()
                .as_uint()
                .unwrap()
                .0,
        )
        .split('.')
        .collect::<Vec<&str>>()
        .first()
        .unwrap()
        .parse::<u64>()
        .unwrap();

        assert_eq!(chain_id, 1); // Provider forked from Mainnet should always have chain ID 1
        assert!(
            (29_900_000..=30_200_000).contains(&gas_limit),
            "{gas_limit}"
        ); // Mainnet gas limit is approx 30m
        assert!((306_276..=306_277).contains(&eth_balance)); // Parity multisig bug affected wallet
                                                             // - balance isn't expected to change
                                                             // significantly
    }

    fn assert_results(results: Vec<StdResult<DynSolValue, Bytes>>) {
        // Get the expected individual results.
        let name = results.get(1).unwrap().as_ref().unwrap().as_str().unwrap();
        let decimals = results
            .get(2)
            .unwrap()
            .as_ref()
            .unwrap()
            .as_uint()
            .unwrap()
            .0
            .to::<u8>();
        let symbol = results.get(3).unwrap().as_ref().unwrap().as_str().unwrap();

        // Assert the returned results are as expected
        assert_eq!(name, "Wrapped Ether");
        assert_eq!(symbol, "WETH");
        assert_eq!(decimals, 18);

        // Also check the calls that were added via the builder pattern
        let name = results.get(5).unwrap().as_ref().unwrap().as_str().unwrap();
        let decimals = results
            .get(6)
            .unwrap()
            .as_ref()
            .unwrap()
            .as_uint()
            .unwrap()
            .0
            .to::<u8>();
        let symbol = results.get(7).unwrap().as_ref().unwrap().as_str().unwrap();
        let chain_id = results
            .get(8)
            .unwrap()
            .as_ref()
            .unwrap()
            .as_uint()
            .unwrap()
            .0
            .to::<u64>();

        assert_eq!(name, "Wrapped Ether");
        assert_eq!(symbol, "WETH");
        assert_eq!(decimals, 18);
        assert_eq!(chain_id, 1);
    }
}
