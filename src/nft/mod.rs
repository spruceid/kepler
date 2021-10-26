use ethcontract::{
    contract,
    web3::types::{Address, U256},
};
use nom::{
    branch,
    bytes::streaming::{tag, take_while_m_n},
    character::is_alphanumeric,
    combinator, multi,
    sequence::tuple,
    IResult,
};
use rocket::futures::{future::ok, TryFuture, TryFutureExt};
use serde_json::Value as JsonLd;
use std::{fmt::Display, ops::Deref, str::FromStr};

pub mod caip19;
pub mod caip2;
pub mod ops;

// type Namespace = ParameterizedString<3, 8>;
struct Namespace(String);
impl Deref for Namespace {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// pub struct ParameterizedString<const MIN: usize, const MAX: usize>(String);

// impl<const MIN: usize, const MAX: usize> Deref for ParameterizedString<MIN, MAX> {
//     type Target = String;
//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }

// impl<const MIN: usize, const MAX: usize> ParameterizedString<MIN, MAX> {
//     fn parser(cond: impl Fn(char) -> bool) -> impl FnMut(&str) -> IResult<&str, Self> {
//         combinator::map(take_while_m_n(MIN, MAX, cond), |s: &str| Self(s.into()))
//     }
// }

fn parse_ns(s: &str) -> IResult<&str, Namespace> {
    // Namespace::parser(|s: char| (s.is_lowercase() && is_alphanumeric(s as u8)) || s == '-')(s)
    combinator::map(
        take_while_m_n(3, 8, |s: char| {
            (s.is_lowercase() && is_alphanumeric(s as u8)) || s == '-'
        }),
        |s: &str| Namespace(s.into()),
    )(s)
}

// type Reference<const MAX: usize> = ParameterizedString<1, MAX>;
struct Reference(String);

impl Deref for Reference {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// pub struct ParameterizedString<const MIN: usize, const MAX: usize>(String);

fn parse_ref(s: &str) -> IResult<&str, Reference> {
    // Reference::parser(|s: char| is_alphanumeric(s as u8) || s == '-')(s)
    combinator::map(
        take_while_m_n(1, 64, |s: char| is_alphanumeric(s as u8) || s == '-'),
        |s: &str| Reference(s.into()),
    )(s)
}

#[derive(Clone)]
pub enum Value {
    Number(U256),
    JsonLd(JsonLd),
    Bool(bool),
    Address(ssi::caip10::BlockchainAccountId),
    Asset(caip19::AssetId),
}

fn parse_value(s: &str) -> IResult<&str, Value> {
    todo!()
}

#[rocket::async_trait]
pub trait Expr {
    type Err;
    async fn execute(self, stack: Stack) -> Result<Stack, Self::Err>;
    fn parse(s: &str) -> IResult<&str, Self>
    where
        Self: Sized;
}

pub enum ExprEnum {
    AssertTrue(ops::AssertTrue),
    GreaterThanOrEqualTo(ops::GreaterThanOrEqualTo),
    GetBalance(ops::GetBalance),
    Push(ops::Push),
}

#[rocket::async_trait]
impl Expr for ExprEnum {
    type Err = anyhow::Error;
    async fn execute(self, stack: Stack) -> Result<Stack, Self::Err> {
        match self {
            Self::AssertTrue(e) => e.execute(stack).await,
            Self::GreaterThanOrEqualTo(e) => e.execute(stack).await,
            Self::GetBalance(e) => e.execute(stack).await,
            Self::Push(e) => e.execute(stack).await,
        }
    }
    fn parse(s: &str) -> IResult<&str, Self> {
        ops::AssertTrue::parse(s)
            .map(|(r, e)| (r, Self::AssertTrue(e)))
            .or_else(|_| {
                ops::GreaterThanOrEqualTo::parse(s).map(|(r, e)| (r, Self::GreaterThanOrEqualTo(e)))
            })
            .or_else(|_| ops::GetBalance::parse(s).map(|(r, e)| (r, Self::GetBalance(e))))
            .or_else(|_| ops::Push::parse(s).map(|(r, e)| (r, Self::Push(e))))
    }
}

async fn execute(s: &str) -> IResult<(), anyhow::Error> {
    multi::fold_many1(ExprEnum::parse, ok(Stack(vec![])), |stack_fut, expr| {
        stack_fut.and_then(|stack| stack.execute(expr))
    })
    .await
}

#[derive(Clone)]
pub struct Stack(Vec<Value>);

impl Stack {
    pub async fn execute<E: Expr>(self, expr: E) -> Result<Self, E::Err> {
        expr.execute(self).await
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
