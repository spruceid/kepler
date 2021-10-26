use super::*;

pub struct Push(Value);

#[rocket::async_trait]
impl Expr for Push {
    type Err = anyhow::Error;
    async fn execute(self, stack: Stack) -> Result<Stack, Self::Err> {
        let mut s = stack.0;
        s.push(self.0);
        Ok(Stack(s))
    }

    fn parse(s: &str) -> IResult<&str, Self>
    where
        Self: Sized,
    {
        combinator::map(tuple((tag("push"), parse_value)), |(_, v)| Self(v))(s)
    }
}

pub struct GetBalance;

#[rocket::async_trait]
impl Expr for GetBalance {
    type Err = anyhow::Error;
    async fn execute(self, stack: Stack) -> Result<Stack, Self::Err> {
        let mut s = stack.0;
        match (s.pop(), s.pop()) {
            (Some(Value::Address(a)), Some(Value::Asset(t))) => {
                todo!()
            }
            _ => Err(anyhow!("Invalid stack parameters or types")),
        }
    }

    fn parse(s: &str) -> IResult<&str, Self>
    where
        Self: Sized,
    {
        combinator::map(tag("get_balance"), |_| Self)(s)
    }
}

pub struct GreaterThanOrEqualTo;

#[rocket::async_trait]
impl Expr for GreaterThanOrEqualTo {
    type Err = anyhow::Error;
    async fn execute(self, stack: Stack) -> Result<Stack, Self::Err> {
        let mut s = stack.0;
        match (s.pop(), s.pop()) {
            (Some(Value::Number(n1)), Some(Value::Number(n2))) => {
                s.push(Value::Bool(n1 >= n2));
                Ok(Stack(s))
            }
            _ => Err(anyhow!("Invalid stack parameters or types")),
        }
    }

    fn parse(s: &str) -> IResult<&str, Self>
    where
        Self: Sized,
    {
        combinator::map(tag(">="), |_| Self)(s)
    }
}

pub struct AssertTrue;

#[rocket::async_trait]
impl Expr for AssertTrue {
    type Err = anyhow::Error;
    async fn execute(self, stack: Stack) -> Result<Stack, Self::Err> {
        let mut s = stack.0;
        match s.pop() {
            Some(Value::Bool(true)) => Ok(Stack(s)),
            Some(Value::Bool(false)) => Err(anyhow!("Assertion Failed")),
            Some(_) => Err(anyhow!("Invalid type")),
            None => Err(anyhow!("Missing Arguement")),
        }
    }

    fn parse(s: &str) -> IResult<&str, Self>
    where
        Self: Sized,
    {
        combinator::map(tag("assert_true"), |_| Self)(s)
    }
}
