use std::convert::TryFrom;

use crate::state::{
    read_config, read_is_migrated, read_pool_info, read_rewards_per_sec, rewards_read,
    rewards_store, stakers_read, store_pool_info, PoolInfo, RewardInfo,
};
use cosmwasm_std::{
    Addr, Api, CanonicalAddr, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Order, Response,
    StdError, StdResult, Storage, Uint128,
};
use oraiswap::asset::{Asset, AssetInfo, AssetRaw};
use oraiswap::querier::calc_range_start;
use oraiswap::staking::{RewardInfoResponse, RewardInfoResponseItem};

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 30;

// deposit_reward must be from reward token contract
pub fn deposit_reward(
    deps: DepsMut,
    info: MessageInfo,
    rewards: Vec<Asset>,
) -> StdResult<Response> {
    let config = read_config(deps.storage)?;

    // only rewarder can execute this message, rewarder may be a contract
    if config.rewarder != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("unauthorized"));
    }

    let mut rewards_amount = Uint128::zero();

    for asset in rewards.iter() {
        let asset_key = asset.info.to_vec(deps.api)?;

        let mut pool_info: PoolInfo = read_pool_info(deps.storage, &asset_key)?;

        let mut normal_reward = asset.amount;

        // normal rewards are array of Assets
        if pool_info.total_bond_amount.is_zero() {
            pool_info.pending_reward += normal_reward;
        } else {
            normal_reward += pool_info.pending_reward;
            let normal_reward_per_bond =
                Decimal::from_ratio(normal_reward, pool_info.total_bond_amount);
            pool_info.reward_index = pool_info.reward_index + normal_reward_per_bond;
            pool_info.pending_reward = Uint128::zero();
        }

        store_pool_info(deps.storage, &asset_key, &pool_info)?;

        rewards_amount += asset.amount;
    }

    Ok(Response::new().add_attributes([
        ("action", "deposit_reward"),
        ("rewards_amount", &rewards_amount.to_string()),
    ]))
}

// withdraw all rewards or single reward depending on asset_token
pub fn withdraw_reward(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    asset_info: Option<AssetInfo>,
) -> StdResult<Response> {
    let staker_addr = deps.api.addr_canonicalize(info.sender.as_str())?;
    let asset_key = asset_info.map_or(None, |a| a.to_vec(deps.api).ok());

    let reward_assets = process_reward_assets(deps.storage, &staker_addr, &asset_key, true)?;

    let messages = reward_assets
        .into_iter()
        .map(|ra| {
            Ok(ra
                .to_normal(deps.api)?
                .into_msg(None, &deps.querier, info.sender.clone())?)
        })
        .collect::<StdResult<Vec<CosmosMsg>>>()?;

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "withdraw_reward"))
}

pub fn withdraw_reward_others(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    staker_addrs: Vec<Addr>,
    asset_info: Option<AssetInfo>,
) -> StdResult<Response> {
    let config = read_config(deps.storage)?;

    // only admin can execute this message
    if config.owner != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("unauthorized"));
    }

    let asset_key = asset_info.map_or(None, |a| a.to_vec(deps.api).ok());
    // let mut messages: Vec<CosmosMsg> = vec![];

    // withdraw reward for each staker
    for staker_addr in staker_addrs {
        let staker_addr_raw = deps.api.addr_canonicalize(staker_addr.as_str())?;
        process_reward_assets(deps.storage, &staker_addr_raw, &asset_key.clone(), false)?;
    }

    Ok(Response::new().add_attribute("action", "withdraw_reward_others"))
}

fn update_reward_assets_amount(reward_assets: &mut Vec<AssetRaw>, rw: AssetRaw, amount: Uint128) {
    match reward_assets.iter_mut().find(|ra| ra.info.eq(&rw.info)) {
        None => {
            reward_assets.push(AssetRaw {
                info: rw.info,
                amount,
            });
        }
        Some(reward_asset) => {
            reward_asset.amount += amount;
        }
    }
}

// this function will return total asset to reward, then later can be updated as pending_withdraw, or send to client
pub fn process_reward_assets(
    storage: &mut dyn Storage,
    staker_addr: &CanonicalAddr,
    asset_key: &Option<Vec<u8>>,
    do_withdraw: bool,
) -> StdResult<Vec<AssetRaw>> {
    let rewards_bucket = rewards_read(storage, staker_addr);

    // single reward withdraw, using Vec to store reference variable in local function
    let reward_pairs = if let Some(asset_key) = asset_key {
        let reward_info = rewards_bucket.may_load(&asset_key)?;
        if let Some(reward_info) = reward_info {
            vec![(asset_key.to_vec(), reward_info)]
        } else {
            vec![]
        }
    } else {
        rewards_bucket
            .range(None, None, Order::Ascending)
            .collect::<StdResult<Vec<(Vec<u8>, RewardInfo)>>>()?
    };

    // only has value when do_withdraw
    let mut reward_assets: Vec<AssetRaw> = vec![];

    for reward_pair in reward_pairs {
        let (asset_key, mut reward_info) = reward_pair;
        let pool_info: PoolInfo = read_pool_info(storage, &asset_key)?;

        // Withdraw reward to pending reward
        // if the lp token was migrated, and the user did not close their position yet, cap the reward at the snapshot
        let pool_index = if pool_info.migration_params.is_some()
            && !read_is_migrated(storage, &asset_key, staker_addr)
        {
            pool_info.migration_params.unwrap().index_snapshot
        } else {
            pool_info.reward_index
        };

        before_share_change(pool_index, &mut reward_info)?;

        if !reward_info.pending_reward.is_zero() {
            // calculate and accumulate the reward amount
            let rewards_per_sec = read_rewards_per_sec(storage, &asset_key)?;
            // now calculate weight
            let total_amount: Uint128 = rewards_per_sec.iter().map(|rw| rw.amount).sum();

            for rw in rewards_per_sec {
                // ignore empty weight
                if rw.amount.is_zero() {
                    continue;
                }
                let amount =
                    reward_info.pending_reward * Decimal::from_ratio(rw.amount, total_amount);

                // update pending_withdraw, first time push it, later update the amount
                update_reward_assets_amount(&mut reward_info.pending_withdraw, rw, amount);
            }

            // reset pending_reward
            reward_info.pending_reward = Uint128::zero();
        }

        // if withdraw, then update reward_assets to create MsgSend
        if do_withdraw {
            for rw in reward_info.pending_withdraw {
                update_reward_assets_amount(&mut reward_assets, rw.clone(), rw.amount);
            }
            reward_info.pending_withdraw = vec![];
        }

        // Update rewards info, if empty bond_amount and withdraw then remove
        if reward_info.bond_amount.is_zero() && do_withdraw {
            rewards_store(storage, staker_addr).remove(&asset_key);
        } else {
            rewards_store(storage, staker_addr).save(&asset_key, &reward_info)?;
        }
    }

    Ok(reward_assets)
}

// withdraw reward to pending reward
pub fn before_share_change(pool_index: Decimal, reward_info: &mut RewardInfo) -> StdResult<()> {
    let pending_reward = (reward_info.bond_amount * pool_index)
        .checked_sub(reward_info.bond_amount * reward_info.index)?;

    reward_info.index = pool_index;
    reward_info.pending_reward += pending_reward;
    Ok(())
}

pub fn query_reward_info(
    deps: Deps,
    staker_addr: Addr,
    asset_info: Option<AssetInfo>,
) -> StdResult<RewardInfoResponse> {
    let staker_addr_raw = deps.api.addr_canonicalize(staker_addr.as_str())?;

    let reward_infos: Vec<RewardInfoResponseItem> =
        _read_reward_infos_response(deps.api, deps.storage, &staker_addr_raw, &asset_info)?;

    Ok(RewardInfoResponse {
        staker_addr,
        reward_infos,
    })
}

pub fn query_all_reward_infos(
    deps: Deps,
    asset_info: AssetInfo,
    start_after: Option<Addr>,
    limit: Option<u32>,
    order: Option<i32>,
) -> StdResult<Vec<RewardInfoResponse>> {
    // default is Ascending
    let order_by = Order::try_from(order.unwrap_or(1))?;
    let asset_key = asset_info.to_vec(deps.api)?;
    let start_after = start_after
        .map_or(None, |a| deps.api.addr_canonicalize(a.as_str()).ok())
        .map(|c| c.to_vec());

    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;

    let (start, end) = match order_by {
        Order::Ascending => (calc_range_start(start_after), None),
        Order::Descending => (None, start_after),
    };

    let info_responses = stakers_read(deps.storage, &asset_key)
        .range(start.as_deref(), end.as_deref(), order_by)
        .take(limit)
        .map(|item| {
            let (k, _) = item?;
            let staker_addr_raw = CanonicalAddr::from(k);
            let reward_infos: Vec<RewardInfoResponseItem> = _read_reward_infos_response(
                deps.api,
                deps.storage,
                &staker_addr_raw,
                &Some(asset_info.clone()),
            )?;
            let staker_addr = deps.api.addr_humanize(&staker_addr_raw)?;
            Ok(RewardInfoResponse {
                staker_addr,
                reward_infos,
            })
        })
        .collect::<StdResult<Vec<RewardInfoResponse>>>()?;

    Ok(info_responses)
}

fn _read_reward_infos_response(
    api: &dyn Api,
    storage: &dyn Storage,
    staker_addr: &CanonicalAddr,
    asset_info: &Option<AssetInfo>,
) -> StdResult<Vec<RewardInfoResponseItem>> {
    let results = _read_reward_infos(api, storage, staker_addr, asset_info)?;
    let reward_infos: Vec<RewardInfoResponseItem> = results
        .into_iter()
        .map(|(asset_info, mut reward_info)| {
            let asset_key = asset_info.to_vec(api)?;
            let pool_info = read_pool_info(storage, &asset_key)?;

            let (pool_index, should_migrate) = if pool_info.migration_params.is_some()
                && !read_is_migrated(storage, &asset_key, staker_addr)
            {
                (
                    pool_info.migration_params.unwrap().index_snapshot,
                    Some(true),
                )
            } else {
                (pool_info.reward_index, None)
            };

            before_share_change(pool_index, &mut reward_info)?;

            let pending_withdraw = reward_info
                .pending_withdraw
                .into_iter()
                .map(|pw| Ok(pw.to_normal(api)?))
                .collect::<StdResult<Vec<Asset>>>()?;

            Ok(RewardInfoResponseItem {
                asset_info: asset_info.to_owned(),
                bond_amount: reward_info.bond_amount,
                pending_reward: reward_info.pending_reward,
                pending_withdraw,
                should_migrate,
            })
        })
        .collect::<StdResult<Vec<RewardInfoResponseItem>>>()?;

    Ok(reward_infos)
}

fn _read_reward_infos(
    api: &dyn Api,
    storage: &dyn Storage,
    staker_addr: &CanonicalAddr,
    asset_info: &Option<AssetInfo>,
) -> StdResult<Vec<(AssetInfo, RewardInfo)>> {
    let rewards_bucket = rewards_read(storage, staker_addr);
    let results: Vec<(AssetInfo, RewardInfo)> = if let Some(asset_info) = asset_info {
        let asset_key = asset_info.to_vec(api)?;

        if let Some(reward_info) = rewards_bucket.may_load(&asset_key)? {
            vec![(asset_info.clone(), reward_info)]
        } else {
            vec![]
        }
    } else {
        rewards_bucket
            .range(None, None, Order::Ascending)
            .map(|item| {
                let (asset_key, reward_info) = item?;

                // try convert to AssetInfo based on reward info
                let asset_info = if reward_info.native_token {
                    AssetInfo::NativeToken {
                        denom: String::from_utf8(asset_key)?,
                    }
                } else {
                    AssetInfo::Token {
                        contract_addr: api.addr_humanize(&asset_key.into())?,
                    }
                };

                Ok((asset_info, reward_info))
            })
            .collect::<StdResult<Vec<(AssetInfo, RewardInfo)>>>()?
    };

    Ok(results)
}
