use agena_plugin_sdk::prelude::*;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Request {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Response {
    value: String,
}

agena_plugin_sdk::plugin_service_endpoint! {
    BadEndpoint {
        service: "test.zero",
        version: 0,
        method: "query",
        input: Request,
        output: Response,
    }
}

fn main() {}
