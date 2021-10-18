use std::collections::HashMap;
use std::convert::TryFrom;

use ed25519_dalek::Verifier;
use nt_abi::FunctionExt;
use nt_utils::TrustMe;
use ton_block::{Deserializable, GetRepresentationHash, MsgAddressInt, Serializable};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

use crate::models::*;
use crate::tokens_object::*;
use crate::utils::*;

mod external;
mod generic_contract;
mod models;
mod tokens_object;
mod transport;
mod utils;

#[wasm_bindgen(js_name = "checkAddress")]
pub fn check_address(address: &str) -> bool {
    nt_utils::validate_address(address)
}

#[wasm_bindgen(js_name = "runLocal")]
pub fn run_local(
    gen_timings: GenTimings,
    last_transaction_id: LastTransactionId,
    account_stuff_boc: &str,
    contract_abi: &str,
    method: &str,
    input: TokensObject,
) -> Result<ExecutionOutput, JsValue> {
    let gen_timings = parse_gen_timings(gen_timings)?;
    let last_transaction_id = parse_last_transaction_id(last_transaction_id)?;
    let account_stuff = parse_account_stuff(account_stuff_boc)?;
    let contract_abi = parse_contract_abi(contract_abi)?;
    let method = contract_abi.function(method).handle_error()?;
    let input = parse_tokens_object(&method.inputs, input).handle_error()?;

    let output = method
        .run_local(account_stuff, gen_timings, &last_transaction_id, &input)
        .handle_error()?;

    make_execution_output(output)
}

#[wasm_bindgen(js_name = "getExpectedAddress")]
pub fn get_expected_address(
    tvc: &str,
    contract_abi: &str,
    workchain_id: i8,
    public_key: Option<String>,
    init_data: TokensObject,
) -> Result<String, JsValue> {
    let mut state_init = ton_block::StateInit::construct_from_base64(tvc).handle_error()?;
    let contract_abi = parse_contract_abi(contract_abi)?;
    let public_key = public_key.as_deref().map(parse_public_key).transpose()?;

    state_init.data = if let Some(data) = state_init.data.take() {
        Some(insert_init_data(&contract_abi, data.into(), &public_key, init_data)?.into_cell())
    } else {
        None
    };

    let hash = state_init.hash().trust_me();

    Ok(MsgAddressInt::AddrStd(ton_block::MsgAddrStd {
        anycast: None,
        workchain_id,
        address: hash.into(),
    })
    .to_string())
}

#[wasm_bindgen(js_name = "packIntoCell")]
pub fn pack_into_cell(params: ParamsList, tokens: TokensObject) -> Result<String, JsValue> {
    let params = parse_params_list(params).handle_error()?;
    let tokens = parse_tokens_object(&params, tokens).handle_error()?;

    let cell = nt_abi::pack_into_cell(&tokens).handle_error()?;
    let bytes = ton_types::serialize_toc(&cell).handle_error()?;
    Ok(base64::encode(&bytes))
}

#[wasm_bindgen(js_name = "unpackFromCell")]
pub fn unpack_from_cell(
    params: ParamsList,
    boc: &str,
    allow_partial: bool,
) -> Result<TokensObject, JsValue> {
    let params = parse_params_list(params).handle_error()?;
    let body = base64::decode(boc).handle_error()?;
    let cell =
        ton_types::deserialize_tree_of_cells(&mut std::io::Cursor::new(body)).handle_error()?;
    nt_abi::unpack_from_cell(&params, cell.into(), allow_partial)
        .handle_error()
        .and_then(make_tokens_object)
}

#[wasm_bindgen(js_name = "extractPublicKey")]
pub fn extract_public_key(boc: &str) -> Result<String, JsValue> {
    crate::utils::parse_account_stuff(boc)
        .and_then(|x| nt_abi::extract_public_key(&x).handle_error())
        .map(hex::encode)
}

#[wasm_bindgen(js_name = "codeToTvc")]
pub fn code_to_tvc(code: &str) -> Result<String, JsValue> {
    let cell = base64::decode(code).handle_error()?;
    ton_types::deserialize_tree_of_cells(&mut std::io::Cursor::new(cell))
        .handle_error()
        .and_then(|x| nt_abi::code_to_tvc(x).handle_error())
        .and_then(|x| x.serialize().handle_error())
        .and_then(|x| ton_types::serialize_toc(&x).handle_error())
        .map(base64::encode)
}

#[wasm_bindgen(js_name = "splitTvc")]
pub fn split_tvc(tvc: &str) -> Result<StateInit, JsValue> {
    let state_init = ton_block::StateInit::construct_from_base64(tvc).handle_error()?;

    let data = match state_init.data {
        Some(data) => {
            let data = ton_types::serialize_toc(&data).handle_error()?;
            Some(base64::encode(data))
        }
        None => None,
    };

    let code = match state_init.code {
        Some(code) => {
            let code = ton_types::serialize_toc(&code).handle_error()?;
            Some(base64::encode(code))
        }
        None => None,
    };

    Ok(ObjectBuilder::new()
        .set("data", data)
        .set("code", code)
        .build()
        .unchecked_into())
}

#[wasm_bindgen(js_name = "encodeInternalInput")]
pub fn encode_internal_input(
    contract_abi: &str,
    method: &str,
    input: TokensObject,
) -> Result<String, JsValue> {
    let contract_abi = parse_contract_abi(contract_abi)?;
    let method = contract_abi.function(method).handle_error()?;
    let input = parse_tokens_object(&method.inputs, input).handle_error()?;

    let body = method
        .encode_input(&Default::default(), &input, true, None)
        .and_then(|value| value.into_cell())
        .handle_error()?;
    let body = ton_types::serialize_toc(&body).handle_error()?;
    Ok(base64::encode(&body))
}

#[wasm_bindgen(js_name = "decodeInput")]
pub fn decode_input(
    message_body: &str,
    contract_abi: &str,
    method: MethodName,
    internal: bool,
) -> Result<Option<DecodedInput>, JsValue> {
    let contract = parse_contract_abi(contract_abi)?;
    let message_body = parse_slice(message_body)?;
    let method = parse_method_name(method)?;
    let (method, data) =
        match nt_abi::decode_input(&contract, message_body, &method, internal).handle_error()? {
            Some(method) => method,
            None => return Ok(None),
        };

    Ok(Some(
        ObjectBuilder::new()
            .set("method", &method.name)
            .set("input", make_tokens_object(data)?)
            .build()
            .unchecked_into(),
    ))
}

#[wasm_bindgen(js_name = "decodeEvent")]
pub fn decode_event(
    message_body: &str,
    contract_abi: &str,
    event: MethodName,
) -> Result<Option<DecodedEvent>, JsValue> {
    let contract = parse_contract_abi(contract_abi)?;
    let message_body = parse_slice(message_body)?;
    let name = parse_method_name(event)?;
    let (event, data) = match nt_abi::decode_event(&contract, message_body, &name).handle_error()? {
        Some(event) => event,
        None => return Ok(None),
    };

    Ok(Some(
        ObjectBuilder::new()
            .set("event", &event.name)
            .set("data", make_tokens_object(data)?)
            .build()
            .unchecked_into(),
    ))
}

#[wasm_bindgen(js_name = "decodeOutput")]
pub fn decode_output(
    message_body: &str,
    contract_abi: &str,
    method: MethodName,
) -> Result<Option<DecodedOutput>, JsValue> {
    let contract = parse_contract_abi(contract_abi)?;
    let message_body = parse_slice(message_body)?;
    let method = parse_method_name(method)?;
    let (method, data) =
        match nt_abi::decode_output(&contract, message_body, &method).handle_error()? {
            Some(method) => method,
            None => return Ok(None),
        };

    Ok(Some(
        ObjectBuilder::new()
            .set("method", &method.name)
            .set("output", make_tokens_object(data)?)
            .build()
            .unchecked_into(),
    ))
}

#[wasm_bindgen(js_name = "decodeTransaction")]
pub fn decode_transaction(
    transaction: Transaction,
    contract_abi: &str,
    method: MethodName,
) -> Result<Option<DecodedTransaction>, JsValue> {
    let transaction: JsValue = transaction.unchecked_into();
    if !transaction.is_object() {
        return Err(TokensJsonError::ObjectExpected).handle_error();
    }

    let contract_abi = parse_contract_abi(contract_abi)?;
    let method = parse_method_name(method)?;

    let in_msg = js_sys::Reflect::get(&transaction, &JsValue::from_str("inMessage"))?;
    if !in_msg.is_object() {
        return Err(TokensJsonError::MessageExpected).handle_error();
    }
    let internal = js_sys::Reflect::get(&in_msg, &JsValue::from_str("src"))?.is_string();

    let body_key = JsValue::from_str("body");
    let in_msg_body = match js_sys::Reflect::get(&in_msg, &body_key)?.as_string() {
        Some(body) => parse_slice(&body)?,
        None => return Ok(None),
    };

    let method = match nt_abi::guess_method_by_input(&contract_abi, &in_msg_body, &method, internal)
        .handle_error()?
    {
        Some(method) => method,
        None => return Ok(None),
    };

    let input = method.decode_input(in_msg_body, internal).handle_error()?;

    let out_msgs = js_sys::Reflect::get(&transaction, &JsValue::from_str("outMessages"))?;
    if !js_sys::Array::is_array(&out_msgs) {
        return Err(TokensJsonError::ArrayExpected).handle_error();
    }

    let dst_key = JsValue::from_str("dst");
    let ext_out_msgs = out_msgs
        .unchecked_into::<js_sys::Array>()
        .iter()
        .filter_map(|message| {
            match js_sys::Reflect::get(&message, &dst_key) {
                Ok(dst) if dst.is_string() => return None,
                Err(error) => return Some(Err(error)),
                _ => {}
            };

            Some(
                match js_sys::Reflect::get(&message, &body_key).map(|item| item.as_string()) {
                    Ok(Some(body)) => parse_slice(&body),
                    Ok(None) => Err(TokensJsonError::MessageBodyExpected).handle_error(),
                    Err(error) => Err(error),
                },
            )
        })
        .collect::<Result<Vec<_>, JsValue>>()?;

    let output = nt_abi::process_raw_outputs(&ext_out_msgs, method).handle_error()?;

    Ok(Some(
        ObjectBuilder::new()
            .set("method", &method.name)
            .set("input", make_tokens_object(input)?)
            .set("output", make_tokens_object(output)?)
            .build()
            .unchecked_into(),
    ))
}

#[wasm_bindgen(js_name = "decodeTransactionEvents")]
pub fn decode_transaction_events(
    transaction: Transaction,
    contract_abi: &str,
) -> Result<DecodedTransactionEvents, JsValue> {
    let transaction: JsValue = transaction.unchecked_into();
    if !transaction.is_object() {
        return Err(TokensJsonError::ObjectExpected).handle_error();
    }

    let contract_abi = parse_contract_abi(contract_abi)?;

    let out_msgs = js_sys::Reflect::get(&transaction, &JsValue::from_str("outMessages"))?;
    if !js_sys::Array::is_array(&out_msgs) {
        return Err(TokensJsonError::ArrayExpected).handle_error();
    }

    let body_key = JsValue::from_str("body");
    let dst_key = JsValue::from_str("dst");
    let ext_out_msgs = out_msgs
        .unchecked_into::<js_sys::Array>()
        .iter()
        .filter_map(|message| {
            match js_sys::Reflect::get(&message, &dst_key) {
                Ok(dst) if dst.is_string() => return None,
                Err(error) => return Some(Err(error)),
                _ => {}
            };

            Some(
                match js_sys::Reflect::get(&message, &body_key).map(|item| item.as_string()) {
                    Ok(Some(body)) => parse_slice(&body),
                    Ok(None) => return None,
                    Err(error) => Err(error),
                },
            )
        })
        .collect::<Result<Vec<_>, JsValue>>()?;

    let events = ext_out_msgs
        .into_iter()
        .filter_map(|body| {
            let id = nt_abi::read_function_id(&body).ok()?;
            let event = contract_abi.event_by_id(id).ok()?;
            let tokens = event.decode_input(body).ok()?;

            let data = match make_tokens_object(tokens) {
                Ok(data) => data,
                Err(e) => return Some(Err(e)),
            };

            Some(Ok(ObjectBuilder::new()
                .set("event", &event.name)
                .set("data", data)
                .build()))
        })
        .collect::<Result<js_sys::Array, JsValue>>()?;

    Ok(events.unchecked_into())
}

#[wasm_bindgen(js_name = "verifySignature")]
pub fn verify_signature(
    public_key: &str,
    data_hash: &str,
    signature: &str,
) -> Result<bool, JsValue> {
    let public_key = parse_public_key(public_key)?;

    let data_hash = match hex::decode(data_hash) {
        Ok(data_hash) => data_hash,
        Err(e) => match base64::decode(data_hash) {
            Ok(data_hash) => data_hash,
            Err(_) => return Err(e).handle_error(),
        },
    };
    if data_hash.len() != 32 {
        return Err("Invalid data hash. Expected 32 bytes").handle_error();
    }

    let signature = match base64::decode(signature) {
        Ok(signature) => signature,
        Err(e) => match hex::decode(signature) {
            Ok(signature) => signature,
            Err(_) => return Err(e).handle_error(),
        },
    };
    let signature = match ed25519_dalek::Signature::try_from(signature.as_slice()) {
        Ok(signature) => signature,
        Err(_) => return Err("Invalid signature. Expected 64 bytes").handle_error(),
    };

    Ok(public_key.verify(&data_hash, &signature).is_ok())
}

#[wasm_bindgen(js_name = "createExternalMessageWithoutSignature")]
pub fn create_unsigned_message_without_signature(
    dst: &str,
    contract_abi: &str,
    method: &str,
    state_init: Option<String>,
    input: TokensObject,
    timeout: u32,
) -> Result<SignedMessage, JsValue> {
    use nt::core::models::{Expiration, ExpireAt};

    // Parse params
    let dst = parse_address(dst)?;
    let contract_abi = parse_contract_abi(contract_abi)?;
    let method = contract_abi.function(method).handle_error()?;
    let state_init = state_init
        .as_deref()
        .map(ton_block::StateInit::construct_from_base64)
        .transpose()
        .handle_error()?;
    let input = parse_tokens_object(&method.inputs, input).handle_error()?;

    // Prepare headers
    let time = chrono::Utc::now().timestamp_millis() as u64;
    let expire_at = ExpireAt::new_from_millis(Expiration::Timeout(timeout), time);

    let mut header = HashMap::with_capacity(3);
    header.insert("time".to_string(), ton_abi::TokenValue::Time(time));
    header.insert(
        "expire".to_string(),
        ton_abi::TokenValue::Expire(expire_at.timestamp),
    );
    header.insert("pubkey".to_string(), ton_abi::TokenValue::PublicKey(None));

    // Encode body
    let body = method
        .encode_input(&header, &input, false, None)
        .handle_error()?;

    // Build message
    let mut message =
        ton_block::Message::with_ext_in_header(ton_block::ExternalInboundMessageHeader {
            dst,
            ..Default::default()
        });
    if let Some(state_init) = state_init {
        message.set_state_init(state_init);
    }
    message.set_body(body.into());

    // Serialize message
    make_signed_message(nt::crypto::SignedMessage {
        message,
        expire_at: expire_at.timestamp,
    })
}
