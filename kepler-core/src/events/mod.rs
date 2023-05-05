pub mod delegation;
pub mod invocation;
pub mod revocation;
pub use kepler_lib::authorization::{KeplerDelegation, KeplerInvocation, KeplerRevocation};

#[derive(Debug)]
pub struct Delegation(pub KeplerDelegation, pub Vec<u8>);

#[derive(Debug)]
pub struct Invocation(pub KeplerInvocation, pub Vec<u8>);

#[derive(Debug)]
pub struct Revocation(pub KeplerRevocation, pub Vec<u8>);

#[derive(Debug)]
pub enum Event {
    Invocation(Invocation),
    Delegation(Delegation),
    Revocation(Revocation),
}
