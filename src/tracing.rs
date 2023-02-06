use opentelemetry::trace::TraceContextExt;
use rocket::{
    fairing::{Fairing, Info, Kind},
    http::Status,
    request::{FromRequest, Outcome},
    Data, Request, Response,
};
use tracing::{field, info_span, subscriber::set_global_default, Span};
use tracing_log::LogTracer;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, Layer, Registry};

use crate::config;

#[derive(Clone)]
pub struct TracingSpan(pub Span);

pub struct TracingFairing {
    pub header_name: String,
}

#[rocket::async_trait]
impl Fairing for TracingFairing {
    fn info(&self) -> Info {
        Info {
            name: "Tracing Fairing",
            kind: Kind::Request | Kind::Response,
        }
    }
    async fn on_request(&self, req: &mut Request<'_>, _data: &mut Data<'_>) {
        let span = info_span!(parent: None, "request", trace_id = field::Empty);
        span.record(
            "trace_id",
            &field::display(&span.context().span().span_context().trace_id()),
        );
        req.local_cache(|| Some(TracingSpan(span)));
    }

    async fn on_response<'r>(&self, req: &'r Request<'_>, res: &mut Response<'r>) {
        if let Some(TracingSpan(span)) = req.local_cache(|| Option::<TracingSpan>::None).to_owned()
        {
            let trace_id = span.context().span().span_context().trace_id();
            res.set_raw_header(self.header_name.clone(), format!("{trace_id}"));
        }
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for TracingSpan {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
        match request.local_cache(|| Option::<TracingSpan>::None) {
            Some(TracingSpan(span)) => Outcome::Success(TracingSpan(span.to_owned())),
            None => Outcome::Failure((Status::InternalServerError, ())),
        }
    }
}

pub fn tracing_try_init(config: &config::Logging) {
    LogTracer::init().unwrap();
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();
    let subscriber = tracing_subscriber::fmt::layer();
    let log = match config.format {
        config::LoggingFormat::Text => subscriber.boxed(),
        config::LoggingFormat::Json => subscriber.json().boxed(),
    };
    let telemetry = if config.tracing.enabled {
        let tracer = opentelemetry_jaeger::new_pipeline()
            .with_service_name("kepler")
            .install_batch(opentelemetry::runtime::Tokio)
            .unwrap();
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        Some(telemetry)
    } else {
        None
    };
    let collector = Registry::default()
        .with(env_filter)
        .with(log)
        .with(telemetry);
    set_global_default(collector).unwrap();
}
