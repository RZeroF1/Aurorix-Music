//! Versioned, bounded bootstrap transport for the public FFI boundary.
//!
//! The complete reviewed facade schema is maintained outside this public
//! repository until it is approved for publication. This module therefore
//! implements the small public-safe envelope in schema/ffi-v1.proto. It is
//! intentionally sufficient to prove version, correlation, oneof, and size
//! handling without exposing Core domain types.

/// The only schema major accepted by this transport.
pub const SCHEMA_MAJOR: u32 = 1;
/// Maximum ordinary request or response envelope size.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// Maximum event envelope size reserved by the transport contract.
pub const MAX_EVENT_BYTES: usize = 256 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_NAMESPACE_BYTES: usize = 128;
const MAX_EXTENSION_SCHEMA_BYTES: usize = 64;
const MAX_EXTENSION_PAYLOAD_BYTES: usize = MAX_MESSAGE_BYTES - 256;

const FIELD_SCHEMA_VERSION: u32 = 1;
const FIELD_REQUEST_ID: u32 = 2;
const FIELD_PING: u32 = 10;
const FIELD_EXTENSION: u32 = 11;
const FIELD_PONG: u32 = 10;
const FIELD_ERROR: u32 = 11;
const MAX_ERROR_CODE_BYTES: usize = 64;
const MAX_ERROR_MESSAGE_BYTES: usize = 1024;

/// One public-safe request body arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestBody {
    /// A harmless liveness request used by generation and ABI smoke tests.
    Ping,
    /// A bounded, namespaced payload reserved for reviewed extensions.
    Extension(ExtensionRequest),
}

/// One public-safe response body arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseBody {
    /// The response to a liveness request.
    Pong,
    /// A bounded, display-safe transport error.
    Error(FfiError),
}

/// A namespaced extension transport projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRequest {
    namespace: String,
    schema_version: String,
    payload: Vec<u8>,
}

impl ExtensionRequest {
    /// Creates a bounded extension request.
    ///
    /// # Errors
    ///
    /// Returns an error when a text field is empty or exceeds its bound, or
    /// when the payload exceeds the extension limit.
    pub fn new(
        namespace: impl Into<String>,
        schema_version: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Self, TransportError> {
        let request = Self {
            namespace: namespace.into(),
            schema_version: schema_version.into(),
            payload: payload.into(),
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the registered extension namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the extension payload schema version.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns the bounded payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_text("extension namespace", &self.namespace, MAX_NAMESPACE_BYTES)?;
        validate_text(
            "extension schema version",
            &self.schema_version,
            MAX_EXTENSION_SCHEMA_BYTES,
        )?;
        if self.payload.len() > MAX_EXTENSION_PAYLOAD_BYTES {
            return Err(TransportError::FieldTooLarge {
                field: "extension payload",
                max: MAX_EXTENSION_PAYLOAD_BYTES,
                actual: self.payload.len(),
            });
        }
        Ok(())
    }
}

/// A typed transport error projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiError {
    code: String,
    message: String,
    retryable: bool,
}

impl FfiError {
    /// Creates a bounded transport error projection.
    ///
    /// # Errors
    ///
    /// Returns an error when the code or message is empty or exceeds its
    /// declared bound.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Result<Self, TransportError> {
        let error = Self {
            code: code.into(),
            message: message.into(),
            retryable,
        };
        validate_text("error code", &error.code, MAX_ERROR_CODE_BYTES)?;
        validate_text("error message", &error.message, MAX_ERROR_MESSAGE_BYTES)?;
        Ok(error)
    }

    /// Returns the machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the bounded display-safe error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether retrying may be appropriate.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

/// A public-safe request envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiRequest {
    schema_version: u32,
    request_id: String,
    body: RequestBody,
}

impl FfiRequest {
    /// Creates a request envelope after validating its contract fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the request ID or extension body is invalid.
    pub fn new(request_id: impl Into<String>, body: RequestBody) -> Result<Self, TransportError> {
        let request = Self {
            schema_version: SCHEMA_MAJOR,
            request_id: request_id.into(),
            body,
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the schema major carried by this envelope.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the caller-generated correlation identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected oneof body arm.
    #[must_use]
    pub const fn body(&self) -> &RequestBody {
        &self.body
    }

    /// Encodes this envelope using the protobuf wire format defined by the
    /// public bootstrap schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope is invalid or exceeds the message
    /// limit.
    pub fn encode(&self) -> Result<Vec<u8>, TransportError> {
        self.validate()?;
        let mut output = Vec::with_capacity(64);
        put_varint_field(
            &mut output,
            FIELD_SCHEMA_VERSION,
            u64::from(self.schema_version),
        );
        put_bytes_field(&mut output, FIELD_REQUEST_ID, self.request_id.as_bytes());
        match &self.body {
            RequestBody::Ping => put_message_field(&mut output, FIELD_PING, &[]),
            RequestBody::Extension(extension) => {
                let mut message = Vec::with_capacity(64 + extension.payload.len());
                put_bytes_field(&mut message, 1, extension.namespace.as_bytes());
                put_bytes_field(&mut message, 2, extension.schema_version.as_bytes());
                put_bytes_field(&mut message, 3, &extension.payload);
                put_message_field(&mut output, FIELD_EXTENSION, &message);
            }
        }
        ensure_message_size(output)
    }

    /// Decodes and validates one protobuf envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed wire data, unknown fields, unsupported
    /// schema versions, invalid oneof arms, or exceeded bounds.
    pub fn decode(input: &[u8]) -> Result<Self, TransportError> {
        if input.len() > MAX_MESSAGE_BYTES {
            return Err(TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: input.len(),
            });
        }

        let mut cursor = Cursor::new(input);
        let mut schema_version = None;
        let mut request_id = None;
        let mut body = None;
        while !cursor.is_empty() {
            let (field, wire_type) = cursor.key()?;
            match field {
                FIELD_SCHEMA_VERSION if wire_type == WireType::Varint => {
                    if schema_version.is_some() {
                        return Err(TransportError::DuplicateField("schema_version"));
                    }
                    schema_version = Some(cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("schema_version exceeds uint32")
                    })?);
                }
                FIELD_REQUEST_ID if wire_type == WireType::LengthDelimited => {
                    if request_id.is_some() {
                        return Err(TransportError::DuplicateField("request_id"));
                    }
                    request_id = Some(cursor.utf8("request_id", MAX_REQUEST_ID_BYTES)?);
                }
                FIELD_PING if wire_type == WireType::LengthDelimited => {
                    select_body(&mut body, RequestBody::Ping)?;
                    let payload = cursor.bytes("ping", 0)?;
                    if !payload.is_empty() {
                        return Err(TransportError::InvalidValue("ping must be empty"));
                    }
                }
                FIELD_EXTENSION if wire_type == WireType::LengthDelimited => {
                    let payload = cursor.bytes("extension", MAX_EXTENSION_PAYLOAD_BYTES + 256)?;
                    if body.is_some() {
                        return Err(TransportError::InvalidValue("multiple body oneof arms"));
                    }
                    select_body(
                        &mut body,
                        RequestBody::Extension(decode_extension(&payload)?),
                    )?;
                }
                FIELD_PING | FIELD_EXTENSION => {
                    return Err(TransportError::InvalidWireType { field });
                }
                _ => return Err(TransportError::UnknownField { field }),
            }
        }

        let request = Self {
            schema_version: schema_version.ok_or(TransportError::MissingField("schema_version"))?,
            request_id: request_id.ok_or(TransportError::MissingField("request_id"))?,
            body: body.ok_or(TransportError::MissingOneof("body"))?,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.schema_version != SCHEMA_MAJOR {
            return Err(TransportError::UnsupportedSchema {
                expected: SCHEMA_MAJOR,
                actual: self.schema_version,
            });
        }
        validate_text("request_id", &self.request_id, MAX_REQUEST_ID_BYTES)?;
        if let RequestBody::Extension(extension) = &self.body {
            extension.validate()?;
        }
        Ok(())
    }
}

/// A public-safe response envelope paired with one request correlation ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiResponse {
    schema_version: u32,
    request_id: String,
    body: ResponseBody,
}

impl FfiResponse {
    /// Creates a response envelope after validating its contract fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the request ID or error body is invalid.
    pub fn new(request_id: impl Into<String>, body: ResponseBody) -> Result<Self, TransportError> {
        let response = Self {
            schema_version: SCHEMA_MAJOR,
            request_id: request_id.into(),
            body,
        };
        response.validate()?;
        Ok(response)
    }

    /// Returns the schema major carried by this envelope.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the request correlation identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected oneof body arm.
    #[must_use]
    pub const fn body(&self) -> &ResponseBody {
        &self.body
    }

    /// Encodes this response using the protobuf wire format.
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope is invalid or exceeds the message
    /// limit.
    pub fn encode(&self) -> Result<Vec<u8>, TransportError> {
        self.validate()?;
        let mut output = Vec::with_capacity(64);
        put_varint_field(
            &mut output,
            FIELD_SCHEMA_VERSION,
            u64::from(self.schema_version),
        );
        put_bytes_field(&mut output, FIELD_REQUEST_ID, self.request_id.as_bytes());
        match &self.body {
            ResponseBody::Pong => put_message_field(&mut output, FIELD_PONG, &[]),
            ResponseBody::Error(error) => {
                let mut message = Vec::with_capacity(64 + error.message.len());
                put_bytes_field(&mut message, 1, error.code.as_bytes());
                put_bytes_field(&mut message, 2, error.message.as_bytes());
                put_varint_field(&mut message, 3, u64::from(error.retryable));
                put_message_field(&mut output, FIELD_ERROR, &message);
            }
        }
        ensure_message_size(output)
    }

    /// Decodes and validates one protobuf response envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed wire data, unknown fields, unsupported
    /// schema versions, invalid oneof arms, or exceeded bounds.
    pub fn decode(input: &[u8]) -> Result<Self, TransportError> {
        if input.len() > MAX_MESSAGE_BYTES {
            return Err(TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: input.len(),
            });
        }

        let mut cursor = Cursor::new(input);
        let mut schema_version = None;
        let mut request_id = None;
        let mut body = None;
        while !cursor.is_empty() {
            let (field, wire_type) = cursor.key()?;
            match field {
                FIELD_SCHEMA_VERSION if wire_type == WireType::Varint => {
                    if schema_version.is_some() {
                        return Err(TransportError::DuplicateField("schema_version"));
                    }
                    schema_version = Some(cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("schema_version exceeds uint32")
                    })?);
                }
                FIELD_REQUEST_ID if wire_type == WireType::LengthDelimited => {
                    if request_id.is_some() {
                        return Err(TransportError::DuplicateField("request_id"));
                    }
                    request_id = Some(cursor.utf8("request_id", MAX_REQUEST_ID_BYTES)?);
                }
                FIELD_PONG if wire_type == WireType::LengthDelimited => {
                    select_response_body(&mut body, ResponseBody::Pong)?;
                    let payload = cursor.bytes("pong", 0)?;
                    if !payload.is_empty() {
                        return Err(TransportError::InvalidValue("pong must be empty"));
                    }
                }
                FIELD_ERROR if wire_type == WireType::LengthDelimited => {
                    let payload = cursor.bytes("error", MAX_ERROR_MESSAGE_BYTES + 256)?;
                    if body.is_some() {
                        return Err(TransportError::InvalidValue(
                            "multiple response body oneof arms",
                        ));
                    }
                    select_response_body(&mut body, ResponseBody::Error(decode_error(&payload)?))?;
                }
                FIELD_PONG | FIELD_ERROR => {
                    return Err(TransportError::InvalidWireType { field });
                }
                _ => return Err(TransportError::UnknownField { field }),
            }
        }

        let response = Self {
            schema_version: schema_version.ok_or(TransportError::MissingField("schema_version"))?,
            request_id: request_id.ok_or(TransportError::MissingField("request_id"))?,
            body: body.ok_or(TransportError::MissingOneof("body"))?,
        };
        response.validate()?;
        Ok(response)
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.schema_version != SCHEMA_MAJOR {
            return Err(TransportError::UnsupportedSchema {
                expected: SCHEMA_MAJOR,
                actual: self.schema_version,
            });
        }
        validate_text("request_id", &self.request_id, MAX_REQUEST_ID_BYTES)?;
        if let ResponseBody::Error(error) = &self.body {
            validate_text("error code", &error.code, MAX_ERROR_CODE_BYTES)?;
            validate_text("error message", &error.message, MAX_ERROR_MESSAGE_BYTES)?;
        }
        Ok(())
    }
}

/// Errors returned when a transport envelope cannot be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The complete wire message exceeded the ordinary envelope limit.
    MessageTooLarge { max: usize, actual: usize },
    /// A text or bytes field exceeded its declared limit.
    FieldTooLarge {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    /// The schema major is not understood by this client.
    UnsupportedSchema { expected: u32, actual: u32 },
    /// A required field was absent.
    MissingField(&'static str),
    /// The required body oneof was absent.
    MissingOneof(&'static str),
    /// The same singular field occurred more than once.
    DuplicateField(&'static str),
    /// A field number was not part of the closed public schema.
    UnknownField { field: u32 },
    /// A known field used an invalid protobuf wire type.
    InvalidWireType { field: u32 },
    /// A scalar or nested message contained an invalid value.
    InvalidValue(&'static str),
    /// A length-delimited value was malformed or truncated.
    Malformed(&'static str),
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MessageTooLarge { max, actual } => {
                write!(formatter, "message is {actual} bytes; limit is {max}")
            }
            Self::FieldTooLarge { field, max, actual } => {
                write!(formatter, "{field} is {actual} bytes; limit is {max}")
            }
            Self::UnsupportedSchema { expected, actual } => {
                write!(
                    formatter,
                    "schema major {actual} is unsupported; expected {expected}"
                )
            }
            Self::MissingField(field) => write!(formatter, "missing required field {field}"),
            Self::MissingOneof(field) => write!(formatter, "missing required oneof {field}"),
            Self::DuplicateField(field) => write!(formatter, "duplicate singular field {field}"),
            Self::UnknownField { field } => write!(formatter, "unknown field {field}"),
            Self::InvalidWireType { field } => {
                write!(formatter, "invalid wire type for field {field}")
            }
            Self::InvalidValue(value) | Self::Malformed(value) => formatter.write_str(value),
        }
    }
}

impl std::error::Error for TransportError {}

fn decode_extension(input: &[u8]) -> Result<ExtensionRequest, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut namespace = None;
    let mut schema_version = None;
    let mut payload = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if namespace.is_some() {
                    return Err(TransportError::DuplicateField("extension namespace"));
                }
                namespace = Some(cursor.utf8("extension namespace", MAX_NAMESPACE_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if schema_version.is_some() {
                    return Err(TransportError::DuplicateField("extension schema version"));
                }
                schema_version =
                    Some(cursor.utf8("extension schema version", MAX_EXTENSION_SCHEMA_BYTES)?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if payload.is_some() {
                    return Err(TransportError::DuplicateField("extension payload"));
                }
                payload = Some(cursor.bytes("extension payload", MAX_EXTENSION_PAYLOAD_BYTES)?);
            }
            1..=3 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    ExtensionRequest::new(
        namespace.ok_or(TransportError::MissingField("extension namespace"))?,
        schema_version.ok_or(TransportError::MissingField("extension schema version"))?,
        payload.ok_or(TransportError::MissingField("extension payload"))?,
    )
}

fn decode_error(input: &[u8]) -> Result<FfiError, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut code = None;
    let mut message = None;
    let mut retryable = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if code.is_some() {
                    return Err(TransportError::DuplicateField("error code"));
                }
                code = Some(cursor.utf8("error code", MAX_ERROR_CODE_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if message.is_some() {
                    return Err(TransportError::DuplicateField("error message"));
                }
                message = Some(cursor.utf8("error message", MAX_ERROR_MESSAGE_BYTES)?);
            }
            3 if wire_type == WireType::Varint => {
                if retryable.is_some() {
                    return Err(TransportError::DuplicateField("error retryable"));
                }
                retryable = Some(cursor.varint()? != 0);
            }
            1..=3 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    FfiError::new(
        code.ok_or(TransportError::MissingField("error code"))?,
        message.ok_or(TransportError::MissingField("error message"))?,
        retryable.ok_or(TransportError::MissingField("error retryable"))?,
    )
}

fn select_body(
    body: &mut Option<RequestBody>,
    selected: RequestBody,
) -> Result<(), TransportError> {
    if body.is_some() {
        return Err(TransportError::InvalidValue("multiple body oneof arms"));
    }
    *body = Some(selected);
    Ok(())
}

fn select_response_body(
    body: &mut Option<ResponseBody>,
    selected: ResponseBody,
) -> Result<(), TransportError> {
    if body.is_some() {
        return Err(TransportError::InvalidValue(
            "multiple response body oneof arms",
        ));
    }
    *body = Some(selected);
    Ok(())
}

fn validate_text(field: &'static str, value: &str, max: usize) -> Result<(), TransportError> {
    if value.is_empty() {
        return Err(TransportError::InvalidValue(field));
    }
    if value.len() > max {
        return Err(TransportError::FieldTooLarge {
            field,
            max,
            actual: value.len(),
        });
    }
    Ok(())
}

fn ensure_message_size(output: Vec<u8>) -> Result<Vec<u8>, TransportError> {
    if output.len() > MAX_MESSAGE_BYTES {
        return Err(TransportError::MessageTooLarge {
            max: MAX_MESSAGE_BYTES,
            actual: output.len(),
        });
    }
    Ok(output)
}

fn put_varint_field(output: &mut Vec<u8>, field: u32, value: u64) {
    put_key(output, field, WireType::Varint);
    put_varint(output, value);
}

fn put_bytes_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
    put_message_field(output, field, value);
}

fn put_message_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
    put_key(output, field, WireType::LengthDelimited);
    put_varint(output, value.len() as u64);
    output.extend_from_slice(value);
}

fn put_key(output: &mut Vec<u8>, field: u32, wire_type: WireType) {
    put_varint(output, (u64::from(field) << 3) | u64::from(wire_type as u8));
}

fn put_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        let byte = u8::try_from(value & 0x7f).expect("varint byte is masked to seven bits");
        output.push(byte | 0x80);
        value >>= 7;
    }
    output.push(u8::try_from(value).expect("final varint byte fits in u8"));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireType {
    Varint = 0,
    LengthDelimited = 2,
}

impl TryFrom<u8> for WireType {
    type Error = TransportError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Varint),
            2 => Ok(Self::LengthDelimited),
            _ => Err(TransportError::Malformed("unsupported protobuf wire type")),
        }
    }
}

struct Cursor<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn is_empty(&self) -> bool {
        self.position == self.input.len()
    }

    fn key(&mut self) -> Result<(u32, WireType), TransportError> {
        let key = self.varint()?;
        let field = u32::try_from(key >> 3)
            .map_err(|_| TransportError::Malformed("field number exceeds uint32"))?;
        if field == 0 {
            return Err(TransportError::Malformed("field number must not be zero"));
        }
        let wire_type = WireType::try_from((key & 0x07) as u8)?;
        Ok((field, wire_type))
    }

    fn varint(&mut self) -> Result<u64, TransportError> {
        let mut value = 0_u64;
        for shift in (0..70).step_by(7) {
            let byte = *self
                .input
                .get(self.position)
                .ok_or(TransportError::Malformed("truncated varint"))?;
            self.position += 1;
            if shift == 63 && byte > 1 {
                return Err(TransportError::Malformed("varint overflows uint64"));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(TransportError::Malformed("varint is too long"))
    }

    fn bytes(&mut self, field: &'static str, max: usize) -> Result<Vec<u8>, TransportError> {
        let length = self.varint()?;
        let length = usize::try_from(length)
            .map_err(|_| TransportError::Malformed("length exceeds usize"))?;
        if length > max {
            return Err(TransportError::FieldTooLarge {
                field,
                max,
                actual: length,
            });
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or(TransportError::Malformed("length overflows input"))?;
        let value = self
            .input
            .get(self.position..end)
            .ok_or(TransportError::Malformed("truncated bytes field"))?;
        self.position = end;
        Ok(value.to_vec())
    }

    fn utf8(&mut self, field: &'static str, max: usize) -> Result<String, TransportError> {
        let value = self.bytes(field, max)?;
        String::from_utf8(value).map_err(|_| TransportError::InvalidValue(field))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExtensionRequest, FfiError, FfiRequest, FfiResponse, MAX_MESSAGE_BYTES, RequestBody,
        ResponseBody, SCHEMA_MAJOR, TransportError,
    };

    #[test]
    fn ping_round_trips_through_the_public_wire_shape() {
        let request = FfiRequest::new("request-1", RequestBody::Ping).expect("request is valid");
        let encoded = request.encode().expect("request encodes");
        let decoded = FfiRequest::decode(&encoded).expect("request decodes");
        assert_eq!(decoded, request);
        assert_eq!(decoded.schema_version(), SCHEMA_MAJOR);
    }

    #[test]
    fn extension_round_trip_preserves_only_bounded_transport_fields() {
        let extension = ExtensionRequest::new("org.example.tool", "1", vec![1, 2, 3])
            .expect("extension is valid");
        let request = FfiRequest::new("request-2", RequestBody::Extension(extension))
            .expect("request is valid");
        let encoded = request.encode().expect("request encodes");
        assert_eq!(
            FfiRequest::decode(&encoded).expect("request decodes"),
            request
        );
    }

    #[test]
    fn unknown_schema_major_fails_closed() {
        let error = FfiRequest::decode(&[0x08, 0x02, 0x12, 0x01, b'x', 0x52, 0x00])
            .expect_err("unknown major must fail");
        assert_eq!(
            error,
            TransportError::UnsupportedSchema {
                expected: SCHEMA_MAJOR,
                actual: 2
            }
        );
    }

    #[test]
    fn unknown_oneof_arm_fails_closed() {
        let error = FfiRequest::decode(&[
            0x08, 0x01, // schema_version
            0x12, 0x01, b'x', // request_id
            0x62, 0x00, // field 12: unknown body arm
        ])
        .expect_err("unknown oneof must fail");
        assert_eq!(error, TransportError::UnknownField { field: 12 });
    }

    #[test]
    fn over_limit_message_fails_before_allocation() {
        let input = vec![0_u8; MAX_MESSAGE_BYTES + 1];
        assert_eq!(
            FfiRequest::decode(&input).expect_err("oversized input must fail"),
            TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: MAX_MESSAGE_BYTES + 1
            }
        );
    }

    #[test]
    fn duplicate_body_arms_fail_closed() {
        let error = FfiRequest::decode(&[0x08, 0x01, 0x12, 0x01, b'x', 0x52, 0x00, 0x5a, 0x00])
            .expect_err("two oneof arms must fail");
        assert_eq!(
            error,
            TransportError::InvalidValue("multiple body oneof arms")
        );
    }

    #[test]
    fn extension_payload_limit_is_enforced() {
        let payload = vec![0_u8; super::MAX_EXTENSION_PAYLOAD_BYTES + 1];
        assert!(matches!(
            ExtensionRequest::new("org.example.tool", "1", payload),
            Err(TransportError::FieldTooLarge {
                field: "extension payload",
                ..
            })
        ));
    }

    #[test]
    fn pong_response_round_trips_with_request_correlation() {
        let response =
            FfiResponse::new("request-1", ResponseBody::Pong).expect("response is valid");
        let encoded = response.encode().expect("response encodes");
        let decoded = FfiResponse::decode(&encoded).expect("response decodes");
        assert_eq!(decoded, response);
    }

    #[test]
    fn typed_error_response_round_trips_and_preserves_retryability() {
        let error = FfiError::new("invalid_request", "the request was rejected", true)
            .expect("error is valid");
        let response =
            FfiResponse::new("request-3", ResponseBody::Error(error)).expect("response is valid");
        let encoded = response.encode().expect("response encodes");
        assert_eq!(
            FfiResponse::decode(&encoded).expect("response decodes"),
            response
        );
    }

    #[test]
    fn unknown_response_oneof_arm_fails_closed() {
        let error = FfiResponse::decode(&[
            0x08, 0x01, // schema_version
            0x12, 0x01, b'x', // request_id
            0x62, 0x00, // field 12: unknown body arm
        ])
        .expect_err("unknown response oneof must fail");
        assert_eq!(error, TransportError::UnknownField { field: 12 });
    }
}
