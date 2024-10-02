use alloy_dyn_abi::DynSolValue;
use alloy_multicall::Multicall;
use alloy_primitives::{address, Bytes};
use alloy_sol_types::sol;
use std::result::Result as StdResult;

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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let rpc_url = "https://rpc.ankr.com/eth".parse().unwrap();
    let provider = alloy_provider::ProviderBuilder::new().on_http(rpc_url);
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
