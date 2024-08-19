use std::{
    pin::Pin,
    task::{Context, Poll},
};

use crate::{stream_helper::map_sender, uds_req_res::UdsResponse};
use futures::StreamExt;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};

pub trait JsonRpcServerTransport<SingleOrBatchRequest: AsRef<SingleOrBatch<JsonRpcRequest>>>:
    futures::Stream<
    Item = (
        SingleOrBatchRequest,
        futures::channel::oneshot::Sender<SingleOrBatch<JsonRpcResponse>>,
    ),
>
{
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(untagged)]
pub enum SingleOrBatch<T> {
    Single(T),
    Batch(Vec<T>),
}

impl<T> SingleOrBatch<T> {
    pub fn map<TOut, MapFn: Fn(T) -> TOut>(self, map_fn: MapFn) -> SingleOrBatch<TOut> {
        match self {
            Self::Single(request) => SingleOrBatch::Single(map_fn(request)),
            Self::Batch(requests) => {
                SingleOrBatch::Batch(requests.into_iter().map(map_fn).collect())
            }
        }
    }
}

impl<T> UdsResponse for SingleOrBatch<T>
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    fn request_parse_error_response() -> Self {
        // TODO: Implement this.
        panic!()
    }
}

pub struct JsonRpcServerStream<
    SingleOrBatchRequest: AsRef<SingleOrBatch<JsonRpcRequest>> + Send + Sync + 'static,
> {
    #[allow(clippy::type_complexity)]
    stream: Pin<
        Box<
            dyn futures::Stream<
                    Item = (
                        SingleOrBatchRequest,
                        futures::channel::oneshot::Sender<SingleOrBatch<JsonRpcResponseData>>,
                    ),
                > + Send,
        >,
    >,
}

impl<SingleOrBatchRequest: AsRef<SingleOrBatch<JsonRpcRequest>> + Send + Sync + 'static>
    futures::Stream for JsonRpcServerStream<SingleOrBatchRequest>
{
    type Item = (
        SingleOrBatchRequest,
        futures::channel::oneshot::Sender<SingleOrBatch<JsonRpcResponseData>>,
    );

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream
            .poll_next_unpin(cx)
            .map(|next_item_or| next_item_or)
    }
}

impl<SingleOrBatchRequest: AsRef<SingleOrBatch<JsonRpcRequest>> + Send + Sync + 'static>
    JsonRpcServerStream<SingleOrBatchRequest>
{
    // TODO: Completely clean up this function. It's a mess.
    pub fn start(
        transport: impl JsonRpcServerTransport<SingleOrBatchRequest> + Send + 'static,
    ) -> Self {
        Self {
            stream: Box::pin(transport.map(|(request, response_sender)| {
                let single_request_id_or = match request.as_ref() {
                    SingleOrBatch::Single(request) => Some(request.id().clone()),
                    SingleOrBatch::Batch(_requests) => None,
                };
                let batch_request_ids_or: Option<Vec<JsonRpcId>> = match request.as_ref() {
                    SingleOrBatch::Single(_request) => None,
                    SingleOrBatch::Batch(requests) => Some(
                        requests
                            .iter()
                            .map(|request| request.id().clone())
                            .collect(),
                    ),
                };

                let response_sender = map_sender(response_sender, |response| match response {
                    SingleOrBatch::Single(response_data) => {
                        let Some(request_id) = single_request_id_or else {
                            panic!("Expected a single request, but got a batch of requests",)
                        };
                        SingleOrBatch::Single(JsonRpcResponse::new(response_data, request_id))
                    }
                    SingleOrBatch::Batch(responses) => {
                        let Some(request_ids) = batch_request_ids_or else {
                            panic!("Expected a batch of requests, but got a single request")
                        };
                        SingleOrBatch::Batch(
                            responses
                                .into_iter()
                                .enumerate()
                                .map(|(i, response_data)| {
                                    let Some(request_id) = request_ids.get(i) else {
                                        panic!("Expected a request at index {i}")
                                    };
                                    JsonRpcResponse::new(response_data, request_id.clone())
                                })
                                .collect(),
                        )
                    }
                });

                (request, response_sender)
            })),
        }
    }
}

// TODO: Uncomment this and have Nip55Client implement it.
// #[async_trait]
// pub trait JsonRpcClientTransport<E> {
//     async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, E>;

//     async fn send_batch_request(
//         &self,
//         requests: Vec<JsonRpcRequest>,
//     ) -> Result<Vec<JsonRpcResponse>, E>;
// }

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
enum JsonRpcVersion {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct JsonRpcRequest {
    jsonrpc: JsonRpcVersion,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<JsonRpcStructuredValue>,
    id: JsonRpcId,
}

impl AsRef<Self> for JsonRpcRequest {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl JsonRpcRequest {
    pub const fn new(
        method: String,
        params: Option<JsonRpcStructuredValue>,
        id: JsonRpcId,
    ) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            method,
            params,
            id,
        }
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub const fn params(&self) -> Option<&JsonRpcStructuredValue> {
        self.params.as_ref()
    }

    pub const fn id(&self) -> &JsonRpcId {
        &self.id
    }
}

// TODO: Rename to `JsonRpcRequestId`.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum JsonRpcId {
    Number(i32),
    String(String),
    Null,
}

impl JsonRpcId {
    fn to_json_value(&self) -> serde_json::Value {
        match self {
            Self::Number(n) => serde_json::Value::Number((*n).into()),
            Self::String(s) => serde_json::Value::String(s.clone()),
            Self::Null => serde_json::Value::Null,
        }
    }
}

impl serde::Serialize for JsonRpcId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_json_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonRpcId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_json::Value::deserialize(deserializer).and_then(|value| {
            if value.is_i64() {
                Ok(Self::Number(
                    i32::try_from(value.as_i64().unwrap()).unwrap(),
                ))
            } else if value.is_string() {
                Ok(Self::String(value.as_str().unwrap().to_string()))
            } else if value.is_null() {
                Ok(Self::Null)
            } else {
                Err(serde::de::Error::custom("Invalid JSON-RPC ID"))
            }
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(untagged)]
pub enum JsonRpcStructuredValue {
    Object(serde_json::Map<String, serde_json::Value>),
    Array(Vec<serde_json::Value>),
}

impl JsonRpcStructuredValue {
    pub fn into_value(self) -> serde_json::Value {
        match self {
            Self::Object(object) => serde_json::Value::Object(object),
            Self::Array(array) => serde_json::Value::Array(array),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct JsonRpcResponse {
    jsonrpc: JsonRpcVersion,
    #[serde(flatten)]
    data: JsonRpcResponseData,
    id: JsonRpcId,
}

impl JsonRpcResponse {
    pub const fn new(data: JsonRpcResponseData, id: JsonRpcId) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            data,
            id,
        }
    }

    pub const fn data(&self) -> &JsonRpcResponseData {
        &self.data
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum JsonRpcResponseData {
    Success { result: serde_json::Value },
    Error { error: JsonRpcError },
}

// TODO: Make these fields private.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct JsonRpcError {
    code: JsonRpcErrorCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl JsonRpcError {
    pub const fn new(
        code: JsonRpcErrorCode,
        message: String,
        data: Option<serde_json::Value>,
    ) -> Self {
        Self {
            code,
            message,
            data,
        }
    }

    pub const fn code(&self) -> JsonRpcErrorCode {
        self.code
    }
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum JsonRpcErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    Custom(i32), // TODO: Make it so that this can only be used for custom error codes, not the standard ones above.
}

impl Serialize for JsonRpcErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let code = match *self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
            Self::Custom(c) => c,
        };
        serializer.serialize_i32(code)
    }
}

impl<'de> Deserialize<'de> for JsonRpcErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let code = i32::deserialize(deserializer)?;
        match code {
            -32700 => Ok(Self::ParseError),
            -32600 => Ok(Self::InvalidRequest),
            -32601 => Ok(Self::MethodNotFound),
            -32602 => Ok(Self::InvalidParams),
            -32603 => Ok(Self::InternalError),
            _ => Ok(Self::Custom(code)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_json_serialization<
        'a,
        T: Serialize + Deserialize<'a> + PartialEq + std::fmt::Debug,
    >(
        value: T,
        json_string: &'a str,
    ) {
        assert_eq!(serde_json::from_str::<T>(json_string).unwrap(), value);
        assert_eq!(serde_json::to_string(&value).unwrap(), json_string);
    }

    #[test]
    fn serialize_and_deserialize_json_rpc_request() {
        // Test with no parameters and null ID.
        assert_json_serialization(
            JsonRpcRequest::new("get_public_key".to_string(), None, JsonRpcId::Null),
            "{\"jsonrpc\":\"2.0\",\"method\":\"get_public_key\",\"id\":null}",
        );

        // Test with object parameters.
        assert_json_serialization(
            JsonRpcRequest::new(
                "get_public_key".to_string(),
                Some(JsonRpcStructuredValue::Object(serde_json::from_str("{\"key_type\":\"rsa\"}").unwrap())),
                JsonRpcId::Null),
            "{\"jsonrpc\":\"2.0\",\"method\":\"get_public_key\",\"params\":{\"key_type\":\"rsa\"},\"id\":null}"
        );

        // Test with array parameters.
        assert_json_serialization(
            JsonRpcRequest::new(
                "fetch_values".to_string(),
                Some(JsonRpcStructuredValue::Array(vec![
                    serde_json::from_str("1").unwrap(),
                    serde_json::from_str("\"2\"").unwrap(),
                    serde_json::from_str("{\"3\":true}").unwrap(),
                ])),
                JsonRpcId::Null,
            ),
            "{\"jsonrpc\":\"2.0\",\"method\":\"fetch_values\",\"params\":[1,\"2\",{\"3\":true}],\"id\":null}",
        );

        // Test with number ID.
        assert_json_serialization(
            JsonRpcRequest::new("get_public_key".to_string(), None, JsonRpcId::Number(1234)),
            "{\"jsonrpc\":\"2.0\",\"method\":\"get_public_key\",\"id\":1234}",
        );

        // Test with number ID.
        assert_json_serialization(
            JsonRpcRequest::new(
                "get_foo_string".to_string(),
                None,
                JsonRpcId::String("foo".to_string()),
            ),
            "{\"jsonrpc\":\"2.0\",\"method\":\"get_foo_string\",\"id\":\"foo\"}",
        );
    }

    #[test]
    fn serialize_and_deserialize_json_rpc_response() {
        // Test with result and null ID.
        assert_json_serialization(
            JsonRpcResponse::new(
                JsonRpcResponseData::Success {
                    result: serde_json::from_str("\"foo\"").unwrap(),
                },
                JsonRpcId::Null,
            ),
            "{\"jsonrpc\":\"2.0\",\"result\":\"foo\",\"id\":null}",
        );

        // Test with error (no data).
        assert_json_serialization(
            JsonRpcResponse::new(
                JsonRpcResponseData::Error {
                    error: JsonRpcError {
                        code: JsonRpcErrorCode::InternalError,
                        message: "foo".to_string(),
                        data: None,
                    },
                },
                JsonRpcId::Null,
            ),
            "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"foo\"},\"id\":null}",
        );

        // Test with error (with data).
        assert_json_serialization(
            JsonRpcResponse::new(
                JsonRpcResponseData::Error {
                    error: JsonRpcError {
                        code: JsonRpcErrorCode::InternalError,
                        message: "foo".to_string(),
                        data: Some(serde_json::from_str("\"bar\"").unwrap()),
                    },
                },
                JsonRpcId::Null,
            ),
            "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"foo\",\"data\":\"bar\"},\"id\":null}",
        );
    }

    #[test]
    fn serialize_deserialize_json_rpc_request_batch() {
        // Test with single request.
        assert_json_serialization(
            SingleOrBatch::Single(JsonRpcRequest::new(
                "get_public_key".to_string(),
                None,
                JsonRpcId::Null,
            )),
            "{\"jsonrpc\":\"2.0\",\"method\":\"get_public_key\",\"id\":null}",
        );

        // Test with batch request.
        assert_json_serialization(
            SingleOrBatch::Batch(vec![
                JsonRpcRequest::new("get_public_key".to_string(), None, JsonRpcId::Null),
                JsonRpcRequest::new("get_foo_string".to_string(), None, JsonRpcId::String("foo".to_string())),
            ]),
            "[{\"jsonrpc\":\"2.0\",\"method\":\"get_public_key\",\"id\":null},{\"jsonrpc\":\"2.0\",\"method\":\"get_foo_string\",\"id\":\"foo\"}]",
        );
    }

    #[test]
    fn serialize_deserialize_json_rpc_response_batch() {
        // Test with single response.
        assert_json_serialization(
            SingleOrBatch::Single(JsonRpcResponse::new(
                JsonRpcResponseData::Success {
                    result: serde_json::from_str("\"foo\"").unwrap(),
                },
                JsonRpcId::Null,
            )),
            "{\"jsonrpc\":\"2.0\",\"result\":\"foo\",\"id\":null}",
        );

        // Test with batch response.
        assert_json_serialization(
            SingleOrBatch::Batch(vec![
                JsonRpcResponse::new(
                    JsonRpcResponseData::Success {
                        result: serde_json::from_str("\"foo\"").unwrap(),
                    },
                    JsonRpcId::Null,
                ),
                JsonRpcResponse::new(
                    JsonRpcResponseData::Success {
                        result: serde_json::from_str("\"bar\"").unwrap(),
                    },
                    JsonRpcId::String("foo".to_string()),
                ),
            ]),
            "[{\"jsonrpc\":\"2.0\",\"result\":\"foo\",\"id\":null},{\"jsonrpc\":\"2.0\",\"result\":\"bar\",\"id\":\"foo\"}]",
        );
    }

    #[test]
    fn serialize_and_deserialize_id() {
        // Test with number ID.
        assert_json_serialization(JsonRpcId::Number(1234), "1234");

        // Test with string ID.
        assert_json_serialization(JsonRpcId::String("foo".to_string()), "\"foo\"");

        // Test with null ID.
        assert_json_serialization(JsonRpcId::Null, "null");
    }

    #[test]
    fn serialize_and_deserialize_error_code() {
        // Test with ParseError.
        assert_json_serialization(JsonRpcErrorCode::ParseError, "-32700");

        // Test with InvalidRequest.
        assert_json_serialization(JsonRpcErrorCode::InvalidRequest, "-32600");

        // Test with MethodNotFound.
        assert_json_serialization(JsonRpcErrorCode::MethodNotFound, "-32601");

        // Test with InvalidParams.
        assert_json_serialization(JsonRpcErrorCode::InvalidParams, "-32602");

        // Test with InternalError.
        assert_json_serialization(JsonRpcErrorCode::InternalError, "-32603");

        // Test with Custom.
        assert_json_serialization(JsonRpcErrorCode::Custom(1234), "1234");
    }
}
