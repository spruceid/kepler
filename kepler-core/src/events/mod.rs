pub mod delegation;
pub mod epoch;
pub mod invocation;
pub mod revocation;

pub use delegation::Delegation;
pub use epoch::Epoch;
pub use invocation::Invocation;
pub use revocation::Revocation;

#[derive(Debug)]
pub enum Event {
    Invocation(Invocation),
    Delegation(Delegation),
    Revocation(Revocation),
}
