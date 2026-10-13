/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! Remote REST runner. `http` and `https` resource gets stay unavailable.

use std::collections::BTreeMap;

use moqui_lib::http::{sim_allows_http, RestClient};
use moqui_lib::object::{to_plain_string, Value};

use crate::context::{Error, ExecutionContext};

/// `http` and `https` resource gets fail with this error.
///
/// The boot test requires `HTTP` and `not available` and must not contact the
/// location. A live fetch would fail that test.
pub fn http_resource_error(location: &str) -> Error {
    let _ = location;
    Error::new("HTTP resources are not available")
}

/// Call a `remote-rest` location. `sim` denies before [`RestClient`] exists.
///
/// `method` overrides `verb`. [`RestClient`] sends GET and POST, so only GET is
/// sent as GET. Every other verb is POST, which is also the default when `verb`
/// is not an HTTP method. `allow_in_sim` stays false.
pub fn run_remote_rest(
    service_name: &str,
    location: &str,
    method: Option<&str>,
    verb: &str,
    parameters: &BTreeMap<String, Value>,
    sim: bool,
) -> Result<BTreeMap<String, Value>, Error> {
    if location.trim().is_empty() {
        return Err(Error::new(format!(
            "Location required to call remote service {service_name}"
        )));
    }
    let http_method = choose_method(method, verb);
    let mut url = expand_location(location, parameters)?;
    if http_method == "GET" {
        url = with_query(&url, parameters);
    }
    if !sim_allows_http(sim, false) {
        return Err(Error::new(format!(
            "HTTP {http_method} {url} denied because sim mode is active"
        )));
    }
    let client = RestClient::new();
    let response = if http_method == "GET" {
        client.get(url).send(false)?
    } else {
        let body = json_scalars(parameters);
        let call = client.post(url);
        if body.is_empty() {
            call.send(false)?
        } else {
            call.json(&body).send(false)?
        }
    };
    let status = response.status();
    if !(200..300).contains(&status) {
        let message = format!("Remote service error {status}: {}", response.body());
        let _ = ExecutionContext::with_current(|ctx| ctx.message().add_error(message.clone()));
        return Err(Error::new(message));
    }
    Ok(response_map(response.body()))
}

fn choose_method(method: Option<&str>, verb: &str) -> &'static str {
    let named = method.map(str::trim).filter(|text| !text.is_empty());
    let raw = named.unwrap_or(verb);
    if raw.eq_ignore_ascii_case("get") {
        "GET"
    } else {
        "POST"
    }
}

fn expand_location(location: &str, parameters: &BTreeMap<String, Value>) -> Result<String, Error> {
    let mut out = String::with_capacity(location.len());
    let mut rest = location;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = &after[..end];
        if let Some(value) = parameters.get(name) {
            out.push_str(&to_plain_string(value));
        } else {
            out.push_str("${");
            out.push_str(name);
            out.push('}');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(moqui_lib::env::expand(&out)?)
}

fn with_query(url: &str, parameters: &BTreeMap<String, Value>) -> String {
    let query = encode_query(parameters);
    if query.is_empty() {
        return url.to_string();
    }
    let joiner = if url.contains('?') { '&' } else { '?' };
    format!("{url}{joiner}{query}")
}

fn encode_query(parameters: &BTreeMap<String, Value>) -> String {
    let mut parts = Vec::new();
    for (key, value) in parameters {
        if matches!(value, Value::Map(_) | Value::List(_)) {
            continue;
        }
        let text = to_plain_string(value);
        if text.is_empty() {
            continue;
        }
        parts.push(format!("{}={}", percent_encode(key), percent_encode(&text)));
    }
    parts.join("&")
}

fn json_scalars(parameters: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    let mut body = BTreeMap::new();
    for (key, value) in parameters {
        if matches!(value, Value::Map(_) | Value::List(_)) {
            continue;
        }
        let text = to_plain_string(value);
        if text.is_empty() {
            continue;
        }
        body.insert(key.clone(), text);
    }
    body
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(char::from(*byte)),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn response_map(body: &str) -> BTreeMap<String, Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return BTreeMap::new();
    }
    match parse_json(trimmed) {
        Ok(Value::Map(map)) => map.into_iter().collect(),
        Ok(other) => {
            let mut map = BTreeMap::new();
            map.insert("response".to_string(), other);
            map
        }
        Err(_) => {
            let mut map = BTreeMap::new();
            map.insert("response".to_string(), Value::string(trimmed));
            map
        }
    }
}

fn parse_json(text: &str) -> Result<Value, Error> {
    let mut parser = Json {
        text: text.as_bytes(),
        index: 0,
    };
    parser.skip();
    let value = parser.value()?;
    parser.skip();
    if parser.index != parser.text.len() {
        return Err(Error::new("trailing JSON"));
    }
    Ok(value)
}

struct Json<'a> {
    text: &'a [u8],
    index: usize,
}

impl Json<'_> {
    fn value(&mut self) -> Result<Value, Error> {
        self.skip();
        let Some(byte) = self.peek() else {
            return Err(Error::new("JSON value is missing"));
        };
        match byte {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Value::string(self.string()?)),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'n' => self.literal(b"null", Value::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(Error::new("JSON value is not valid")),
        }
    }

    fn object(&mut self) -> Result<Value, Error> {
        self.bump(b'{')?;
        let map = moqui_lib::context_stack::ContextStack::new();
        let frame = map.shared_map();
        self.skip();
        if self.peek() == Some(b'}') {
            self.index += 1;
            drop(map);
            return Ok(unwrap_frame(frame));
        }
        loop {
            self.skip();
            let key = self.string()?;
            self.skip();
            self.bump(b':')?;
            let value = self.value()?;
            frame.borrow_mut().insert(key, value);
            self.skip();
            match self.peek() {
                Some(b',') => self.index += 1,
                Some(b'}') => {
                    self.index += 1;
                    break;
                }
                _ => return Err(Error::new("JSON object is not closed")),
            }
        }
        drop(map);
        Ok(unwrap_frame(frame))
    }

    fn array(&mut self) -> Result<Value, Error> {
        self.bump(b'[')?;
        let mut items = Vec::new();
        self.skip();
        if self.peek() == Some(b']') {
            self.index += 1;
            return Ok(Value::List(items));
        }
        loop {
            items.push(self.value()?);
            self.skip();
            match self.peek() {
                Some(b',') => self.index += 1,
                Some(b']') => {
                    self.index += 1;
                    break;
                }
                _ => return Err(Error::new("JSON array is not closed")),
            }
        }
        Ok(Value::List(items))
    }

    fn string(&mut self) -> Result<String, Error> {
        self.bump(b'"')?;
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            self.index += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escaped = self.peek().ok_or_else(|| Error::new("JSON string escape is missing"))?;
                    self.index += 1;
                    match escaped {
                        b'"' | b'\\' | b'/' => out.push(char::from(escaped)),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.hex_char()?),
                        _ => return Err(Error::new("JSON string escape is not valid")),
                    }
                }
                _ => out.push(char::from(byte)),
            }
        }
        Err(Error::new("JSON string is not closed"))
    }

    fn hex_char(&mut self) -> Result<char, Error> {
        if self.index + 4 > self.text.len() {
            return Err(Error::new("JSON unicode escape is short"));
        }
        let hex = std::str::from_utf8(&self.text[self.index..self.index + 4])
            .map_err(|_| Error::new("JSON unicode escape is not hex"))?;
        let code = u32::from_str_radix(hex, 16).map_err(|_| Error::new("JSON unicode escape is not hex"))?;
        self.index += 4;
        char::from_u32(code).ok_or_else(|| Error::new("JSON unicode escape is not a character"))
    }

    fn number(&mut self) -> Result<Value, Error> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')) {
            self.index += 1;
        }
        let text =
            std::str::from_utf8(&self.text[start..self.index]).map_err(|_| Error::new("JSON number is not utf-8"))?;
        if let Ok(number) = text.parse::<i64>() {
            return Ok(Value::I64(number));
        }
        if let Ok(number) = text.parse::<f64>() {
            return Ok(Value::F64(number));
        }
        Err(Error::new(format!("JSON number {text} is not valid")))
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Result<Value, Error> {
        if self.text[self.index..].starts_with(word) {
            self.index += word.len();
            Ok(value)
        } else {
            Err(Error::new("JSON literal is not valid"))
        }
    }

    fn bump(&mut self, expected: u8) -> Result<(), Error> {
        self.skip();
        if self.peek() == Some(expected) {
            self.index += 1;
            Ok(())
        } else {
            Err(Error::new(format!("JSON expected {}", char::from(expected))))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.index).copied()
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }
}

fn unwrap_frame(frame: moqui_lib::context_stack::Frame) -> Value {
    Value::Map(match std::rc::Rc::try_unwrap(frame) {
        Ok(cell) => cell.into_inner(),
        Err(frame) => frame.borrow().clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, Instant};

    use moqui_lib::object::Value;

    use super::run_remote_rest;

    #[test]
    fn http_resource_error_names_http_and_not_available() {
        let err = super::http_resource_error("https://example.com/");
        let shown = err.to_string();
        assert!(shown.contains("HTTP"), "{shown}");
        assert!(shown.contains("not available"), "{shown}");
    }

    #[test]
    fn sim_denies_before_a_socket_is_required() {
        let err = run_remote_rest(
            "example.send#Ping",
            "http://127.0.0.1:1/no-such-remote",
            Some("GET"),
            "send",
            &BTreeMap::new(),
            true,
        )
        .expect_err("sim denies");
        let shown = err.to_string();
        assert!(shown.contains("sim mode is active"), "{shown}");
        assert!(shown.contains("http://127.0.0.1:1/no-such-remote"), "{shown}");
        assert!(!shown.to_ascii_lowercase().contains("refused"), "{shown}");
        assert!(!shown.contains("failed"), "{shown}");
    }

    #[test]
    fn remote_rest_reads_the_stub_body() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = thread::spawn(move || serve_one(listener));
        let mut parameters = BTreeMap::new();
        parameters.insert("note".to_string(), Value::string("kept"));
        let result = run_remote_rest(
            "example.get#Item",
            &format!("http://127.0.0.1:{port}/items/${{note}}"),
            None,
            "get",
            &parameters,
            false,
        )
        .expect("stub response");
        let body = server.join().expect("server thread");
        assert!(body.contains("GET /items/kept"), "{body}");
        assert_eq!(result.get("enumTypeId").and_then(Value::as_str), Some("DayOfWeek"));
        assert_eq!(result.get("ok").and_then(Value::as_str), Some("yes"));
    }

    fn serve_one(listener: TcpListener) -> String {
        listener.set_nonblocking(true).expect("nonblocking");
        let started = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if started.elapsed() > Duration::from_secs(5) {
                        return String::from("accept timed out");
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => return format!("accept failed: {err}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        let request = read_headers(&mut stream);
        let payload = r#"{"ok":"yes","enumTypeId":"DayOfWeek"}"#;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(payload.as_bytes());
        let _ = stream.flush();
        request
    }

    fn read_headers(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0_u8; 1024];
        while !buf.windows(4).any(|window| window == b"\r\n\r\n") && buf.len() < 65_536 {
            match stream.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(size) => buf.extend_from_slice(&tmp[..size]),
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }
}
