use super::*;

pub struct ChainId {
    namespace: Namespace,
    reference: Reference,
}

pub fn parse(s: &str) -> IResult<&str, ChainId> {
    tuple((parse_ns, tag(":"), parse_ref))(s).map(|(rest, (namespace, _, reference))| {
        (
            rest,
            ChainId {
                namespace,
                reference,
            },
        )
    })
}

impl FromStr for ChainId {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}

impl Display for ChainId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.namespace.0, self.reference.0)
    }
}
