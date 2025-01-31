use cosmwasm_schema::cw_serde;

use crate::error::ContractError;
use cosmwasm_std::{Coin, Uint128};
use cw20::Cw20Coin;
use std::convert::TryInto;

#[cw_serde]
pub enum Amount {
    Native(Coin),
    // FIXME? USe Cw20CoinVerified, and validate cw20 addresses
    Cw20(Cw20Coin),
}

impl Amount {
    pub fn from_parts(denom: String, amount: Uint128) -> Self {
        if denom.starts_with("cw20:") {
            let address = denom.get(5..).unwrap().into();
            Amount::Cw20(Cw20Coin { address, amount })
        } else {
            Amount::Native(Coin { denom, amount })
        }
    }

    pub fn cw20(amount: u128, addr: &str) -> Self {
        Amount::Cw20(Cw20Coin {
            address: addr.into(),
            amount: Uint128::new(amount),
        })
    }

    pub fn native(amount: u128, denom: &str) -> Self {
        Amount::Native(Coin {
            denom: denom.to_string(),
            amount: Uint128::new(amount),
        })
    }
}

impl Amount {
    pub fn denom(&self) -> String {
        match self {
            Amount::Native(c) => c.denom.clone(),
            Amount::Cw20(c) => get_cw20_denom(c.address.as_str()),
        }
    }

    pub fn amount(&self) -> Uint128 {
        match self {
            Amount::Native(c) => c.amount,
            Amount::Cw20(c) => c.amount,
        }
    }

    /// convert the amount into u64
    pub fn u64_amount(&self) -> Result<u64, ContractError> {
        Ok(self.amount().u128().try_into()?)
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Amount::Native(c) => c.amount.is_zero(),
            Amount::Cw20(c) => c.amount.is_zero(),
        }
    }
}

pub fn get_cw20_denom(contract: &str) -> String {
    format!("cw20:{}", contract)
}

#[cfg(test)]
mod test {
    use crate::amount::Amount;
    use cosmwasm_std::Uint128;

    #[test]
    fn parse_amount() {
        // native denom
        let res = Amount::from_parts("ucosm".to_string(), 1u8.into());

        assert_eq!("ucosm", res.denom());
        assert_eq!(Uint128::new(1), res.amount());

        // cw20 token
        let res = Amount::from_parts("cw20:my-token".to_string(), 1u8.into());

        assert_eq!("cw20:my-token", res.denom());
        assert_eq!(Uint128::new(1), res.amount());
    }
}
