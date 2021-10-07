use ethcontract::{
    contract,
    web3::{
        types::{Address, U256},
        Web3,
    },
};

pub mod caip2 {
    pub struct ChainId {
        namespace: String,
        reference: String,
    }
}

pub mod caip19 {
    use super::*;
    use nom::{character::complete::alphanumeric1, character::is_alphanumeric};
    use std::{fmt::Display, str::FromStr};

    pub struct AssetType {
        chain_id: caip2::ChainId,
        asset_namespace: String,
        asset_reference: String,
    }

    impl FromStr for AssetType {
        type Err = anyhow::Error;
        fn from_str(s: &str) -> Result<Self, Self::Error> {}
    }

    pub struct AssetId {
        asset_type: AssetType,
        token_id: String,
    }
}

#[rocket::async_trait]
pub trait TokenInterface {
    type AccountInfo;
    type Denomination;
    type Error;
    async fn balance(&self, account: &Self::AccountInfo)
        -> Result<Self::Denomination, Self::Error>;
}

#[rocket::async_trait]
impl TokenInterface for erc20::Contract {
    type Denomination = U256;
    type Error = anyhow::Error;
    type AccountInfo = Address;
    async fn balance(
        &self,
        account: &Self::AccountInfo,
    ) -> Result<Self::Denomination, Self::Error> {
        Ok(self.balance_of(*account).call().await?)
    }
}

#[rocket::async_trait]
impl TokenInterface for erc721::Contract {
    type Denomination = U256;
    type Error = anyhow::Error;
    type AccountInfo = Address;
    async fn balance(
        &self,
        account: &Self::AccountInfo,
    ) -> Result<Self::Denomination, Self::Error> {
        Ok(self.balance_of(*account).call().await?)
    }
}

pub struct ERC1155(erc1155::Contract, pub U256);

#[rocket::async_trait]
impl TokenInterface for ERC1155 {
    type Denomination = U256;
    type Error = anyhow::Error;
    type AccountInfo = Address;
    async fn balance(
        &self,
        account: &Self::AccountInfo,
    ) -> Result<Self::Denomination, Self::Error> {
        Ok(self.0.balance_of(*account, self.1).call().await?)
    }
}

contract!(
    pub "src/nft/ierc20.json",
    mod = erc20,
    methods {
        balanceOf(address) as balance_of;
    }
);

contract!(
    pub "src/nft/ierc721.json",
    mod = erc721,
    methods {
        balanceOf(address) as balance_of;
        safeTransferFrom(address,address,uint256,bytes) as safe_transfer_from_with_data;
    }
);

contract!(
    pub "src/nft/ierc1155.json",
    mod = erc1155,
    methods {
        balanceOf(address,uint256) as balance_of;
    }
);
