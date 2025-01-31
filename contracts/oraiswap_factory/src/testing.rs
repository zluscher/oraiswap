use oraiswap::asset::{AssetInfo, PairInfo};

use oraiswap::create_entry_points_testing;
use oraiswap::pair::DEFAULT_COMMISSION_RATE;
use oraiswap::querier::query_pair_info_from_pair;
use oraiswap::testing::MockApp;

#[test]
fn create_pair() {
    let mut app = MockApp::new(&[]);
    app.set_token_contract(Box::new(create_entry_points_testing!(oraiswap_token)));
    app.set_oracle_contract(Box::new(create_entry_points_testing!(oraiswap_oracle)));

    app.set_factory_and_pair_contract(
        Box::new(create_entry_points_testing!(crate).with_reply(crate::contract::reply)),
        Box::new(
            create_entry_points_testing!(oraiswap_pair).with_reply(oraiswap_pair::contract::reply),
        ),
    );

    let contract_addr1 = app.create_token("assetA");
    let contract_addr2 = app.create_token("assetB");

    let asset_infos = [
        AssetInfo::Token {
            contract_addr: contract_addr1,
        },
        AssetInfo::Token {
            contract_addr: contract_addr2,
        },
    ];

    // create pair
    let contract_addr = app.create_pair(asset_infos.clone()).unwrap();

    // query pair info
    let pair_info = query_pair_info_from_pair(&app.as_querier(), contract_addr.clone()).unwrap();

    // should never change commission rate once deployed
    let pair_res = app.query_pair(asset_infos.clone()).unwrap();
    assert_eq!(
        pair_res,
        PairInfo {
            oracle_addr: app.oracle_addr,
            liquidity_token: pair_info.liquidity_token,
            contract_addr,
            asset_infos,
            commission_rate: DEFAULT_COMMISSION_RATE.into()
        }
    );
}
