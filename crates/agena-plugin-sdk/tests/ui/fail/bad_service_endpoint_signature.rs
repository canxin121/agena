use agena_plugin_sdk::prelude::*;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Request {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct WrongRequest {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Response {
    value: String,
}

agena_plugin_sdk::plugin_service_endpoint! {
    QueryEndpoint {
        service: "test.endpoint",
        version: 1,
        method: "query",
        input: Request,
        output: Response,
    }
}

#[derive(Default)]
struct BadPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad_endpoint_signature",
    version = "0.0.0",
    summary = "compile fail fixture"
)]
impl BadPlugin {
    #[service(QueryEndpoint)]
    fn query(&self, input: &WrongRequest) -> Response {
        Response { value: input.value.clone() }
    }
}

fn main() {}
