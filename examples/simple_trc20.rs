use alloy_dyn_abi::DynSolValue;
use alloy_multicall::Multicall;
use alloy_primitives::{address, Bytes};
use alloy_sol_types::sol;
use std::result::Result as StdResult;

sol! {
    #[derive(Debug, PartialEq)]
    #[sol(rpc, abi, extra_methods)]
    interface TRC20 {
        function totalSupply() external view returns (uint256 totalSupply);
        function balanceOf(address owner) external view returns (uint256 balance);
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let rpc_url = "https://rpc.ankr.com/tron_jsonrpc".parse().unwrap();
    let provider = alloy_provider::ProviderBuilder::new().on_http(rpc_url);
    // Tron use base58 encoded, below is hex encoded value of `TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t`
    let usdc_contract_addr = address!("a614f803B6FD780986A42c78Ec9c7f77e6DeD13C");

    // Create the multicall instance
    let mut multicall = Multicall::with_provider_chain_id(provider.clone())
        .await
        .unwrap();

    // Generate the TRC20 instance we'll be using to create the individual calls
    let functions = TRC20::abi::functions();

    // Create the individual calls
    let name_call = functions.get("name").unwrap().first().unwrap();
    let total_supply_call = functions.get("totalSupply").unwrap().first().unwrap();
    let decimals_call = functions.get("decimals").unwrap().first().unwrap();
    let symbol_call = functions.get("symbol").unwrap().first().unwrap();

    // Add the calls
    multicall.add_call(usdc_contract_addr, total_supply_call, &[], true);
    multicall.add_call(usdc_contract_addr, name_call, &[], true);
    multicall.add_call(usdc_contract_addr, decimals_call, &[], true);
    multicall.add_call(usdc_contract_addr, symbol_call, &[], true);

    // Add the same calls via the builder pattern
    multicall
        .with_call(usdc_contract_addr, name_call, &[], true)
        .with_call(usdc_contract_addr, name_call, &[], true)
        .with_call(usdc_contract_addr, decimals_call, &[], true)
        .with_call(usdc_contract_addr, symbol_call, &[], true)
        .add_get_chain_id();

    // MulticallV3
    //
    // Similar to multicall.set_version(3); multicall.call().await.unwrap();
    // but tron JSON RPC require `data` field instead of `input` field
    // so we need to map `input` to `data` before calling `call()`
    let call_builder = multicall.as_aggregate_3();
    let agg_result = call_builder
        // TODO: this is needed because tron don't accept `input` field ony `data`
        // https://github.com/ethereum/go-ethereum/issues/15628
        // not sure if here is better way to avoid doing what `call()` is doing internally
        .map(|mut req| {
            req.input.data = req.input.input.take();
            req
        })
        .call()
        .await
        .unwrap();

    let results = multicall
        .parse_multicall_result(agg_result.returnData)
        .unwrap();

    assert_results(results);
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
    assert_eq!(name, "Tether USD");
    assert_eq!(symbol, "USDT");
    assert_eq!(decimals, 6);

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

    assert_eq!(name, "Tether USD");
    assert_eq!(symbol, "USDT");
    assert_eq!(decimals, 6);
    assert_eq!(chain_id, 728126428);
}
