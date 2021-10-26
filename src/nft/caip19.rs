use super::*;

pub struct AssetType {
    chain_id: caip2::ChainId,
    namespace: Namespace,
    reference: Reference,
}

pub fn parse_type(s: &str) -> IResult<&str, AssetType> {
    tuple((caip2::parse, tag("/"), parse_ns, tag(":"), parse_ref))(s).map(
        |(rest, (chain_id, _, namespace, _, reference))| {
            (
                rest,
                AssetType {
                    chain_id,
                    namespace,
                    reference,
                },
            )
        },
    )
}

impl FromStr for AssetType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}

impl Display for AssetType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}/{}:{}",
            &self.chain_id, self.namespace.0, self.reference.0
        )
    }
}

pub struct AssetId {
    asset_type: AssetType,
    token_id: Reference,
}

pub fn parse_id(s: &str) -> IResult<&str, AssetId> {
    tuple((parse_type, tag("/"), parse_ref))(s).map(|(rest, (asset_type, _, token_id))| {
        (
            rest,
            AssetId {
                asset_type,
                token_id,
            },
        )
    })
}

impl Display for AssetId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{}", &self.asset_type, self.token_id.0)
    }
}
