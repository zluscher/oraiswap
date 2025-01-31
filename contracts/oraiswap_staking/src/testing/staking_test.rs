use crate::contract::{execute, instantiate, query};
use cosmwasm_std::testing::{
    mock_dependencies, mock_dependencies_with_balance, mock_env, mock_info,
};
use cosmwasm_std::{
    attr, coin, from_binary, to_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, StdError, SubMsg,
    Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};
use oraiswap::asset::{Asset, AssetInfo, ORAI_DENOM};
use oraiswap::create_entry_points_testing;
use oraiswap::pair::PairResponse;
use oraiswap::staking::{
    Cw20HookMsg, ExecuteMsg, InstantiateMsg, PoolInfoResponse, QueryMsg, RewardInfoResponse,
    RewardInfoResponseItem,
};
use oraiswap::testing::{AttributeUtil, MockApp, ATOM_DENOM};

#[test]
fn test_bond_tokens() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        owner: Some(Addr::unchecked("owner")),
        rewarder: Addr::unchecked("rewarder"),
        minter: Some(Addr::unchecked("mint")),
        oracle_addr: Addr::unchecked("oracle"),
        factory_addr: Addr::unchecked("factory"),
        base_denom: None,
    };

    let info = mock_info("addr", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::RegisterAsset {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset"),
        },
        staking_token: Addr::unchecked("staking"),
    };

    let info = mock_info("owner", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: "addr".to_string(),
        amount: Uint128::from(100u128),
        msg: to_binary(&Cw20HookMsg::Bond {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        })
        .unwrap(),
    });

    let info = mock_info("staking", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    let data = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::RewardInfo {
            asset_info: Some(AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            }),
            staker_addr: Addr::unchecked("addr"),
        },
    )
    .unwrap();
    let res: RewardInfoResponse = from_binary(&data).unwrap();
    assert_eq!(
        res,
        RewardInfoResponse {
            staker_addr: Addr::unchecked("addr"),
            reward_infos: vec![RewardInfoResponseItem {
                asset_info: AssetInfo::Token {
                    contract_addr: Addr::unchecked("asset")
                },
                pending_reward: Uint128::zero(),
                pending_withdraw: vec![],
                bond_amount: Uint128::from(100u128),
                should_migrate: None,
            }],
        }
    );

    let data = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::PoolInfo {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        },
    )
    .unwrap();

    let pool_info: PoolInfoResponse = from_binary(&data).unwrap();
    assert_eq!(
        pool_info,
        PoolInfoResponse {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset")
            },
            staking_token: Addr::unchecked("staking"),
            total_bond_amount: Uint128::from(100u128),
            reward_index: Decimal::zero(),
            pending_reward: Uint128::zero(),
            migration_deprecated_staking_token: None,
            migration_index_snapshot: None,
        }
    );

    // bond 100 more tokens from other account
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: "addr2".to_string(),
        amount: Uint128::from(100u128),
        msg: to_binary(&Cw20HookMsg::Bond {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        })
        .unwrap(),
    });
    let info = mock_info("staking", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let data = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::PoolInfo {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        },
    )
    .unwrap();
    let pool_info: PoolInfoResponse = from_binary(&data).unwrap();
    assert_eq!(
        pool_info,
        PoolInfoResponse {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset")
            },
            staking_token: Addr::unchecked("staking"),
            total_bond_amount: Uint128::from(200u128),
            reward_index: Decimal::zero(),
            pending_reward: Uint128::zero(),
            migration_deprecated_staking_token: None,
            migration_index_snapshot: None,
        }
    );

    // failed with unauthorized
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: "addr".to_string(),
        amount: Uint128::from(100u128),
        msg: to_binary(&Cw20HookMsg::Bond {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        })
        .unwrap(),
    });

    let info = mock_info("staking2", &[]);
    let res = execute(deps.as_mut(), mock_env(), info, msg);
    match res {
        Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "unauthorized"),
        _ => panic!("Must return unauthorized error"),
    }
}

#[test]
fn test_unbond() {
    let mut deps = mock_dependencies_with_balance(&[
        coin(10000000000u128, ORAI_DENOM),
        coin(20000000000u128, ATOM_DENOM),
    ]);

    let msg = InstantiateMsg {
        owner: Some(Addr::unchecked("owner")),
        rewarder: Addr::unchecked("rewarder"),
        minter: Some(Addr::unchecked("mint")),
        oracle_addr: Addr::unchecked("oracle"),
        factory_addr: Addr::unchecked("factory"),
        base_denom: None,
    };

    let info = mock_info("addr", &[]);
    let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    // will also add to the index the pending rewards from before the migration
    let msg = ExecuteMsg::UpdateRewardsPerSec {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset"),
        },
        assets: vec![
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ORAI_DENOM.to_string(),
                },
                amount: 100u128.into(),
            },
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ATOM_DENOM.to_string(),
                },
                amount: 200u128.into(),
            },
        ],
    };
    let info = mock_info("owner", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // register asset
    let msg = ExecuteMsg::RegisterAsset {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset"),
        },
        staking_token: Addr::unchecked("staking"),
    };

    let info = mock_info("owner", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // bond 100 tokens
    let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender: "addr".to_string(),
        amount: Uint128::from(100u128),
        msg: to_binary(&Cw20HookMsg::Bond {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        })
        .unwrap(),
    });
    let info = mock_info("staking", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    let msg = ExecuteMsg::DepositReward {
        rewards: vec![Asset {
            info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
            amount: Uint128::from(300u128),
        }],
    };
    let info = mock_info("rewarder", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info.clone(), msg.clone()).unwrap();

    // will also add to the index the pending rewards from before the migration
    let msg = ExecuteMsg::UpdateRewardsPerSec {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset"),
        },
        assets: vec![
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ORAI_DENOM.to_string(),
                },
                amount: 100u128.into(),
            },
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ATOM_DENOM.to_string(),
                },
                amount: 100u128.into(),
            },
        ],
    };
    let info = mock_info("owner", &[]);
    let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    // unbond 150 tokens; failed
    let msg = ExecuteMsg::Unbond {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset"),
        },
        amount: Uint128::from(150u128),
    };

    let info = mock_info("addr", &[]);
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
    match res {
        StdError::GenericErr { msg, .. } => {
            assert_eq!(msg, "Cannot unbond more than bond amount");
        }
        _ => panic!("Must return generic error"),
    };

    // normal unbond
    let msg = ExecuteMsg::Unbond {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset"),
        },
        amount: Uint128::from(100u128),
    };

    let info = mock_info("addr", &[]);
    let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(
        res.messages,
        vec![
            SubMsg::new(WasmMsg::Execute {
                contract_addr: "staking".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: "addr".to_string(),
                    amount: Uint128::from(100u128),
                })
                .unwrap(),
                funds: vec![],
            }),
            SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: "addr".to_string(),
                amount: vec![coin(99u128, ORAI_DENOM)],
            })),
            SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: "addr".to_string(),
                amount: vec![coin(199u128, ATOM_DENOM)],
            }))
        ]
    );

    let data = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::PoolInfo {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset"),
            },
        },
    )
    .unwrap();
    let pool_info: PoolInfoResponse = from_binary(&data).unwrap();
    assert_eq!(
        pool_info,
        PoolInfoResponse {
            asset_info: AssetInfo::Token {
                contract_addr: Addr::unchecked("asset")
            },
            staking_token: Addr::unchecked("staking"),
            total_bond_amount: Uint128::zero(),
            reward_index: Decimal::from_ratio(300u128, 100u128),
            pending_reward: Uint128::zero(),
            migration_deprecated_staking_token: None,
            migration_index_snapshot: None,
        }
    );

    let data = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::RewardInfo {
            asset_info: None,
            staker_addr: Addr::unchecked("addr"),
        },
    )
    .unwrap();
    let res: RewardInfoResponse = from_binary(&data).unwrap();
    assert_eq!(
        res,
        RewardInfoResponse {
            staker_addr: Addr::unchecked("addr"),
            reward_infos: vec![],
        }
    );
}

#[test]
fn test_auto_stake() {
    let mut app = MockApp::new(&[(&"addr".to_string(), &[coin(10000000000u128, ORAI_DENOM)])]);

    app.set_oracle_contract(Box::new(create_entry_points_testing!(oraiswap_oracle)));

    app.set_token_contract(Box::new(create_entry_points_testing!(oraiswap_token)));

    app.set_factory_and_pair_contract(
        Box::new(
            create_entry_points_testing!(oraiswap_factory)
                .with_reply(oraiswap_factory::contract::reply),
        ),
        Box::new(
            create_entry_points_testing!(oraiswap_pair).with_reply(oraiswap_pair::contract::reply),
        ),
    );

    let asset_addr = app.create_token("asset");
    let reward_addr = app.create_token("reward");
    // update other contract token balance
    app.set_token_balances(&[
        (
            &"reward".to_string(),
            &[(&"addr".to_string(), &Uint128::from(10000000000u128))],
        ),
        (
            &"asset".to_string(),
            &[(&"addr".to_string(), &Uint128::from(10000000000u128))],
        ),
    ]);

    let asset_infos = [
        AssetInfo::NativeToken {
            denom: ORAI_DENOM.to_string(),
        },
        AssetInfo::Token {
            contract_addr: asset_addr.clone(),
        },
    ];

    // create pair
    let pair_addr = app.create_pair(asset_infos.clone()).unwrap();

    let PairResponse { info: pair_info } = app
        .query(pair_addr.clone(), &oraiswap::pair::QueryMsg::Pair {})
        .unwrap();

    // set allowance
    app.execute(
        Addr::unchecked("addr"),
        asset_addr.clone(),
        &cw20::Cw20ExecuteMsg::IncreaseAllowance {
            spender: pair_addr.to_string(),
            amount: Uint128::from(100u128),
            expires: None,
        },
        &[],
    )
    .unwrap();

    // provide liquidity
    // successfully provide liquidity for the exist pool
    let msg = oraiswap::pair::ExecuteMsg::ProvideLiquidity {
        assets: [
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ORAI_DENOM.to_string(),
                },
                amount: Uint128::from(100u128),
            },
            Asset {
                info: AssetInfo::Token {
                    contract_addr: asset_addr.clone(),
                },
                amount: Uint128::from(100u128),
            },
        ],
        slippage_tolerance: None,
        receiver: None,
    };

    let _res = app
        .execute(
            Addr::unchecked("addr"),
            pair_addr.clone(),
            &msg,
            &[Coin {
                denom: ORAI_DENOM.to_string(),
                amount: Uint128::from(100u128),
            }],
        )
        .unwrap();

    let code_id = app.upload(Box::new(create_entry_points_testing!(crate)));

    let msg = InstantiateMsg {
        owner: Some(Addr::unchecked("owner")),
        rewarder: reward_addr.clone(),
        minter: Some(Addr::unchecked("mint")),
        oracle_addr: app.oracle_addr.clone(),
        factory_addr: app.factory_addr.clone(),
        base_denom: None,
    };

    let staking_addr = app
        .instantiate(code_id, Addr::unchecked("addr"), &msg, &[], "staking")
        .unwrap();

    // set allowance
    app.execute(
        Addr::unchecked("addr"),
        asset_addr.clone(),
        &cw20::Cw20ExecuteMsg::IncreaseAllowance {
            spender: staking_addr.to_string(),
            amount: Uint128::from(100u128),
            expires: None,
        },
        &[],
    )
    .unwrap();

    let msg = ExecuteMsg::RegisterAsset {
        asset_info: AssetInfo::Token {
            contract_addr: asset_addr.clone(),
        },
        staking_token: pair_info.liquidity_token.clone(),
    };

    let _res = app
        .execute(Addr::unchecked("owner"), staking_addr.clone(), &msg, &[])
        .unwrap();

    // no token asset
    let msg = ExecuteMsg::AutoStake {
        assets: [
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ORAI_DENOM.to_string(),
                },
                amount: Uint128::from(100u128),
            },
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ORAI_DENOM.to_string(),
                },
                amount: Uint128::from(100u128),
            },
        ],
        slippage_tolerance: None,
    };

    let res = app.execute(
        Addr::unchecked("addr"),
        staking_addr.clone(),
        &msg,
        &[Coin {
            denom: ORAI_DENOM.to_string(),
            amount: Uint128::from(100u128),
        }],
    );

    app.assert_fail(res);

    // no native asset
    let msg = ExecuteMsg::AutoStake {
        assets: [
            Asset {
                info: AssetInfo::Token {
                    contract_addr: asset_addr.clone(),
                },
                amount: Uint128::from(1u128),
            },
            Asset {
                info: AssetInfo::Token {
                    contract_addr: asset_addr.clone(),
                },
                amount: Uint128::from(1u128),
            },
        ],
        slippage_tolerance: None,
    };

    let res = app.execute(Addr::unchecked("addr"), staking_addr.clone(), &msg, &[]);

    app.assert_fail(res);

    let msg = ExecuteMsg::AutoStake {
        assets: [
            Asset {
                info: AssetInfo::NativeToken {
                    denom: ORAI_DENOM.to_string(),
                },
                amount: Uint128::from(100u128),
            },
            Asset {
                info: AssetInfo::Token {
                    contract_addr: asset_addr.clone(),
                },
                amount: Uint128::from(1u128),
            },
        ],
        slippage_tolerance: None,
    };

    // attempt with no coins
    let res = app.execute(Addr::unchecked("addr"), staking_addr.clone(), &msg, &[]);
    app.assert_fail(res);

    let _res = app
        .execute(
            Addr::unchecked("addr"),
            staking_addr.clone(),
            &msg,
            &[Coin {
                denom: ORAI_DENOM.to_string(),
                amount: Uint128::from(100u128),
            }],
        )
        .unwrap();

    // wrong asset
    let msg = ExecuteMsg::AutoStakeHook {
        asset_info: AssetInfo::Token {
            contract_addr: Addr::unchecked("asset1"),
        },
        staking_token: pair_info.liquidity_token.clone(),
        staker_addr: Addr::unchecked("addr"),
        prev_staking_token_amount: Uint128::zero(),
    };
    let res = app.execute(staking_addr.clone(), staking_addr.clone(), &msg, &[]);
    // pool not found error
    app.assert_fail(res);

    // valid msg
    let msg = ExecuteMsg::AutoStakeHook {
        asset_info: AssetInfo::Token {
            contract_addr: asset_addr.clone(),
        },
        staking_token: pair_info.liquidity_token.clone(),
        staker_addr: Addr::unchecked("addr"),
        prev_staking_token_amount: Uint128::zero(),
    };

    // unauthorized attempt
    let res = app.execute(Addr::unchecked("addr"), staking_addr.clone(), &msg, &[]);
    app.assert_fail(res);

    // successfull attempt

    let res = app
        .execute(staking_addr.clone(), staking_addr.clone(), &msg, &[])
        .unwrap();
    assert_eq!(
        // first attribute is _contract_addr
        res.get_attributes(1),
        vec![
            attr("action", "bond"),
            attr("staker_addr", "addr"),
            attr("asset_info", asset_addr.as_str()),
            attr("amount", "1"),
        ]
    );

    let pool_info: PoolInfoResponse = app
        .query(
            staking_addr.clone(),
            &QueryMsg::PoolInfo {
                asset_info: AssetInfo::Token {
                    contract_addr: asset_addr.clone(),
                },
            },
        )
        .unwrap();

    assert_eq!(
        pool_info,
        PoolInfoResponse {
            asset_info: AssetInfo::Token {
                contract_addr: asset_addr.clone()
            },
            staking_token: pair_info.liquidity_token.clone(),
            total_bond_amount: Uint128::from(2u128),
            reward_index: Decimal::zero(),
            pending_reward: Uint128::zero(),
            migration_deprecated_staking_token: None,
            migration_index_snapshot: None,
        }
    );
}
