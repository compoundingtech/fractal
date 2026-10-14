//! Stateless Streamable HTTP prototype. No shell, terminal or daemon-control socket access.
use axum::{Router, http::StatusCode, routing::get};
use rmcp::{ServerHandler, RoleServer, ErrorData, model::*, service::RequestContext};
use rmcp::transport::streamable_http_server::{StreamableHttpService, StreamableHttpServerConfig, session::local::LocalSessionManager};
use rmcp::ServiceExt;
use clap::Parser;
use serde_json::{Value, json};
use std::{path::PathBuf, net::SocketAddr};

#[derive(Parser)]
struct Args {
    #[arg(long)] socket: PathBuf,
    #[arg(long)] credential_file: PathBuf,
    #[arg(long, default_value="http", value_parser=["http","stdio"])] transport: String,
    #[arg(long, default_value="127.0.0.1:8787")] listen: SocketAddr,
}
#[derive(Clone)]
struct Adapter { client: reqwest::Client, bearer: String }
const TOOLS: &[(&str, &str, bool)] = &[
    ("st_alerts_list", "List owner alert identifiers and counts.", true),
    ("st_alert_get", "Read one exact owner alert, including its episode and named answer options.", true),
    ("st_alert_answer", "Propose one typed person-ask answer. Requires separate owner approval in st; retry identical arguments after approval.", false),
    ("st_missions_list", "List mission metadata.", true),
    ("st_agents_list", "List agent metadata.", true),
    ("st_conversation_read", "Read one bounded conversation content range. Content may contain sensitive material.", true),
    ("st_message_send", "Send one exact message to an owner-allowlisted agent, authorized by grant scope and the ChatGPT confirmation card.", false),
];
fn schema(name: &str) -> Value {
    let props = match name {
        "st_alert_answer" => json!({"action_id":{"type":"string"},"idempotency_key":{"type":"string"},"fence":{"type":"object"},"alert_id":{"type":"string"},"episode":{"type":"string"},"option":{"type":"string"},"summary":{"type":"string"}}),
        "st_message_send" => json!({"action_id":{"type":"string"},"idempotency_key":{"type":"string"},"fence":{"type":"object"},"to":{"type":"string"},"title":{"type":"string"},"content":{"type":"string"}}),
        "st_alert_get" => json!({"alert_id":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_conversation_read" => json!({"agent_id":{"type":"string"},"session_id":{"type":"string"},"reference":{"type":"string"},"revision":{"type":"string"},"start":{"type":"integer"},"end":{"type":"integer"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        _ => json!({"limit":{"type":"integer","minimum":1},"cursor":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
    };
    let required = match name {
        "st_alert_answer" => vec!["action_id","idempotency_key","fence","alert_id","episode","option","summary"],
        "st_message_send" => vec!["action_id","idempotency_key","fence","to","title","content"],
        "st_alert_get" => vec!["alert_id"],
        "st_conversation_read" => vec!["agent_id","session_id","reference","revision","start","end"],
        _ => vec![],
    };
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}
async fn call(adapter: &Adapter, name: &str, args: &Value) -> Result<Value,Value> {
    if !TOOLS.iter().any(|(tool,_,_)| *tool == name) { return Err(json!({"code":"unknown-tool"})); }
    let contract = schema(name);
    let object = args.as_object().ok_or_else(|| json!({"code":"invalid-arguments"}))?;
    if object.keys().any(|key| contract["properties"].get(key).is_none()) || contract["required"].as_array().unwrap().iter().any(|key| !object.contains_key(key.as_str().unwrap())) {
        return Err(json!({"code":"invalid-arguments","message":"Use the exact published tool schema."}));
    }
    let write = matches!(name,"st_alert_answer"|"st_message_send");
    let route = match name {"st_alerts_list"=>"attention-rich","st_alert_get"=>"attention-content","st_missions_list"=>"missions-rich","st_agents_list"=>"agents-rich","st_conversation_read"=>"conversation-content",_=>"propose"};
    let url = format!("http://localhost/v1/client/assistant/{route}");
    let request = if write {
        let parameters = if name == "st_alert_answer" {
            json!({"target_id":args["alert_id"],"episode":args["episode"],"summary":args["summary"],"answer":{"id":args["option"]}})
        } else {json!({"to":args["to"],"title":args["title"],"content":args["content"]})};
        adapter.client.post(url).json(&json!({"api_version":"st3.client.v0","id":args["action_id"],"type":if name=="st_alert_answer" {"work.done"} else {"message.send"},"idempotency_key":args["idempotency_key"],"fence":args["fence"],"parameters":parameters}))
    } else {
        let query: Vec<_> = object.iter().map(|(k,v)| (k.as_str(),v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string()))).collect();
        adapter.client.get(url).query(&query)
    };
    let response = request.bearer_auth(&adapter.bearer).send().await.map_err(|_| json!({"code":"unavailable","message":"The paired st daemon is unavailable."}))?;
    let status = response.status();
    let value = response.json::<Value>().await.map_err(|_|json!({"code":"invalid-daemon-response"}))?;
    if status.is_success() {Ok(value)} else {Err(json!({"code":if status.as_u16()==413 {"incomplete"} else {"daemon-denied"},"http_status":status.as_u16(),"daemon":value}))}
}
impl ServerHandler for Adapter {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("st-chatgpt",env!("CARGO_PKG_VERSION")))
    }
    async fn list_tools(&self, _:Option<PaginatedRequestParams>, _:RequestContext<RoleServer>) -> Result<ListToolsResult,ErrorData> {
        let tools = TOOLS.iter().map(|(name,description,read)| serde_json::from_value(json!({
            "name":name,"description":description,"inputSchema":schema(name),
            "annotations":{"readOnlyHint":read,"destructiveHint":*name=="st_alert_answer","idempotentHint":true,"openWorldHint":*name=="st_message_send"}
        })).expect("static tool definition")).collect();
        Ok(ListToolsResult { tools, ..Default::default() })
    }
    async fn call_tool(&self, request:CallToolRequestParams, _:RequestContext<RoleServer>) -> Result<CallToolResponse,ErrorData> {
        let args=request.arguments.map(Value::Object).unwrap_or_else(||json!({}));
        let (value,error)=match call(self,&request.name,&args).await {Ok(v)=>(v,false),Err(v)=>(v,true)};
        let result=if error {CallToolResult::error(vec![ContentBlock::text(value.to_string())])}
            else {CallToolResult::success(vec![ContentBlock::text(value.to_string())])};
        Ok(result.into())
    }
}
#[tokio::main]
async fn main() -> Result<(),Box<dyn std::error::Error>> {
    let args=Args::parse();
    if args.socket.file_name().and_then(|name|name.to_str()) != Some("st3-client.sock") { return Err("only the paired gateway st3-client.sock is permitted".into()); }
    if !args.listen.ip().is_loopback() { return Err("the prototype MCP listener must be loopback-only".into()); }
    let bearer=std::fs::read_to_string(args.credential_file)?.trim().to_owned();
    let client=reqwest::Client::builder().unix_socket(args.socket).build()?;
    let adapter=Adapter {client,bearer};
    if args.transport == "stdio" {
        adapter.serve(rmcp::transport::stdio()).await?.waiting().await?;
        return Ok(());
    }
    let service=StreamableHttpService::new(move ||Ok(adapter.clone()),std::sync::Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_legacy_session_mode(false).with_json_response(true));
    let app=Router::new().route("/mcp",get(||async {StatusCode::METHOD_NOT_ALLOWED}).post_service(service))
        .fallback(||async {StatusCode::NOT_FOUND});
    axum::serve(tokio::net::TcpListener::bind(args.listen).await?,app).await?;
    Ok(())
}
