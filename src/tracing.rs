use rocket::http::Status;
use rocket::request::FromRequest;
use rocket::request::Outcome;
use rocket::serde::{json::Json, Serialize};
use rocket::{
    fairing::{Fairing, Info, Kind},
    Data, Request, Response,
};

use opentelemetry::trace::TraceContextExt;
use tracing::{field, info_span, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
// use tracing_log::LogTracer;

// use tracing_subscriber::Layer;
// use tracing_subscriber::{registry::LookupSpan, EnvFilter};
use uuid::Uuid;
// use yansi::Paint;

// Spans

#[derive(Clone, Debug)]
pub struct RequestId<T = String>(pub T);

// // Allows a route to access the request id
// #[rocket::async_trait]
// impl<'r> FromRequest<'r> for RequestId {
//     type Error = ();

//     async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
//         match &*request.local_cache(|| RequestId::<Option<String>>(None)) {
//             RequestId(Some(request_id)) => Outcome::Success(RequestId(request_id.to_owned())),
//             RequestId(None) => Outcome::Failure((Status::InternalServerError, ())),
//         }
//     }
// }

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
        // let user_agent = req.headers().get_one("User-Agent").unwrap_or("");
        // let trace_id = req
        //     .headers()
        //     .get_one(&self.header_name)
        //     .map(ToString::to_string)
        //     .unwrap_or_else(|| Uuid::new_v4().to_string());

        // let mut carrier = HashMap::from([("trace_id", trace_id)]);
        // let propagator = opentelemetry_jaeger::Propagator::new();
        // let parent_context = propagator.extract(&carrier);

        // req.local_cache(|| Some(RequestId(request_id.to_owned())));

        let span = info_span!(
            parent: None,
            "request",
            // otel.name=%format!("{} {}", req.method(), req.uri().path()),
            // http.method = %req.method(),
            // http.uri = %req.uri().path(),
            // http.user_agent=%user_agent,
            // http.status_code = tracing::field::Empty,
            // http.request_id=%request_id,
            // trace_id=%trace_id
            trace_id = field::Empty
        );
        // span.set_parent(parent_context.clone());
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
            // let _entered_span = span.entered();
            // _entered_span.record("http.status_code", &res.status().code);

            // if let Some(request_id) = &req.local_cache(|| RequestId::<Option<String>>(None)).0 {
            //     info!("Returning request {} with {}", request_id, res.status());
            // }
            res.set_raw_header(self.header_name.clone(), format!("{}", trace_id));

            // drop(_entered_span);
        }

        // if let Some(request_id) = &req.local_cache(|| RequestId::<Option<String>>(None)).0 {
        //     res.set_raw_header(self.header_name.clone(), request_id);
        // }
    }
}

// Allows a route to access the span
#[rocket::async_trait]
impl<'r> FromRequest<'r> for TracingSpan {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
        match &*request.local_cache(|| Option::<TracingSpan>::None) {
            Some(TracingSpan(span)) => Outcome::Success(TracingSpan(span.to_owned())),
            None => Outcome::Failure((Status::InternalServerError, ())),
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OutputData<'a> {
    pub message: &'a str,
    pub request_id: String,
}

// #[get("/abc")]
// pub async fn abc<'a>(
//     span: TracingSpan,
//     request_id: RequestId,
// ) -> Result<Json<OutputData<'a>>, (Status, Json<OutputData<'a>>)> {
//     let entered = span.0.enter();
//     info!("Hello World");

//     let mock_data = OutputData {
//         message: "Hello World",
//         request_id: request_id.0,
//     };
//     span.0.record(
//         "output",
//         &serde_json::to_string(&mock_data).unwrap().as_str(),
//     );
//     drop(entered);
//     Err((Status::NotFound, Json(mock_data)))
// }

// // Logging

// use tracing_subscriber::field::MakeExt;

// pub enum LogType {
//     Formatted,
//     Json,
// }

// impl From<String> for LogType {
//     fn from(input: String) -> Self {
//         match input.as_str() {
//             "formatted" => Self::Formatted,
//             "json" => Self::Json,
//             _ => panic!("Unkown log type {}", input),
//         }
//     }
// }

// pub fn default_logging_layer<S>() -> impl Layer<S>
// where
//     S: tracing::Subscriber,
//     S: for<'span> LookupSpan<'span>,
// {
//     let field_format = tracing_subscriber::fmt::format::debug_fn(|writer, field, value| {
//         // We'll format the field name and value separated with a colon.
//         if field.name() == "message" {
//             write!(writer, "{:?}", Paint::new(value).bold())
//         } else {
//             write!(writer, "{}: {:?}", field, Paint::default(value).bold())
//         }
//     })
//     .delimited(", ")
//     .display_messages();

//     tracing_subscriber::fmt::layer()
//         .fmt_fields(field_format)
//         // Configure the formatter to use `print!` rather than
//         // `stdout().write_str(...)`, so that logs are captured by libtest's test
//         // capturing.
//         .with_test_writer()
// }

// pub fn json_logging_layer<
//     S: for<'a> tracing_subscriber::registry::LookupSpan<'a> + tracing::Subscriber,
// >() -> impl tracing_subscriber::Layer<S> {
//     Paint::disable();

//     tracing_subscriber::fmt::layer()
//         .json()
//         // Configure the formatter to use `print!` rather than
//         // `stdout().write_str(...)`, so that logs are captured by libtest's test
//         // capturing.
//         .with_test_writer()
// }

// #[derive(PartialEq, Eq, Debug, Clone, Copy)]
// pub enum LogLevel {
//     /// Only shows errors and warnings: `"critical"`.
//     Critical,
//     /// Shows errors, warnings, and some informational messages that are likely
//     /// to be relevant when troubleshooting such as configuration: `"support"`.
//     Support,
//     /// Shows everything except debug and trace information: `"normal"`.
//     Normal,
//     /// Shows everything: `"debug"`.
//     Debug,
//     /// Shows nothing: "`"off"`".
//     Off,
// }

// impl From<&str> for LogLevel {
//     fn from(s: &str) -> Self {
//         return match &*s.to_ascii_lowercase() {
//             "critical" => LogLevel::Critical,
//             "support" => LogLevel::Support,
//             "normal" => LogLevel::Normal,
//             "debug" => LogLevel::Debug,
//             "off" => LogLevel::Off,
//             _ => panic!("a log level (off, debug, normal, support, critical)"),
//         };
//     }
// }

// pub fn filter_layer(level: LogLevel) -> EnvFilter {
//     let filter_str = match level {
//         LogLevel::Critical => "warn,hyper=off,rustls=off",
//         LogLevel::Support => "warn,rocket::support=info,hyper=off,rustls=off",
//         LogLevel::Normal => "info,hyper=off,rustls=off",
//         LogLevel::Debug => "trace",
//         LogLevel::Off => "off",
//     };

//     tracing_subscriber::filter::EnvFilter::try_new(filter_str).expect("filter string must parse")
// }

// // Rocket setup

// #[launch]
// fn rocket() -> _ {
//     use tracing_subscriber::prelude::*;

//     LogTracer::init().expect("Unable to setup log tracer!");

//     let log_type =
//         LogType::from(std::env::var("LOG_TYPE").unwrap_or_else(|_| "formatted".to_string()));
//     let log_level = LogLevel::from(
//         std::env::var("LOG_LEVEL")
//             .unwrap_or_else(|_| "normal".to_string())
//             .as_str(),
//     );

//     match log_type {
//         LogType::Formatted => {
//             tracing::subscriber::set_global_default(
//                 tracing_subscriber::registry()
//                     .with(default_logging_layer())
//                     .with(filter_layer(log_level)),
//             )
//             .unwrap();
//         }
//         LogType::Json => {
//             tracing::subscriber::set_global_default(
//                 tracing_subscriber::registry()
//                     .with(json_logging_layer())
//                     .with(filter_layer(log_level)),
//             )
//             .unwrap();
//         }
//     };

//     rocket::build()
//         .mount("/", routes![abc])
//         .attach(TracingFairing)
// }
