use rocket_governor::{Method, Quota, RocketGovernable};

pub struct RateLimitGuard;

impl<'r> RocketGovernable<'r> for RateLimitGuard {
    fn quota(_method: Method, _route_name: &str) -> Quota {
        // TODO: make this configurable
        Quota::per_second(Self::nonzero(10u32))
    }
}
