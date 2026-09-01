use super::*;
use base64::engine::general_purpose::STANDARD;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

fn test_vault_scope() -> ContentDigest {
    ContentDigest::parse("ab".repeat(32)).unwrap()
}

#[derive(Debug, Clone)]
struct ObservedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
}

fn spawn_image_server(
    responses: Vec<(u16, String)>,
) -> (String, Arc<Mutex<Vec<ObservedRequest>>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let thread_observed = observed.clone();
    let handle = std::thread::spawn(move || {
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("image test server failed to accept request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "request closed before headers");
                request.extend_from_slice(&buffer[..read]);
                if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "request closed before body");
                request.extend_from_slice(&buffer[..read]);
            }
            let request_line = headers.lines().next().unwrap();
            let mut request_parts = request_line.split_whitespace();
            let method = request_parts.next().unwrap().to_string();
            let path = request_parts.next().unwrap().to_string();
            let authorization = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then(|| value.trim().to_string())
            });
            thread_observed.lock().unwrap().push(ObservedRequest {
                method,
                path,
                authorization,
                body: String::from_utf8(request[header_end..header_end + content_length].to_vec())
                    .unwrap(),
            });
            let reason = if status == 200 { "OK" } else { "Error" };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .ok();
            stream.flush().ok();
        }
    });
    (format!("http://{address}/api/v1"), observed, handle)
}

fn image_catalog(resolution: &str, output_formats: &[&str]) -> String {
    serde_json::json!({
        "data": [{
            "id": "author/model",
            "name": "Image Model",
            "description": "A test raster model",
            "created": 1_800_000_000,
            "architecture": {
                "input_modalities": ["text"],
                "output_modalities": ["image"]
            },
            "supported_parameters": {
                "resolution": {"type": "enum", "values": [resolution]},
                "aspect_ratio": {"type": "enum", "values": ["1:1"]},
                "n": {"type": "range", "min": 1, "max": 1},
                "output_format": {"type": "enum", "values": output_formats}
            },
            "supports_streaming": false,
            "endpoints": "/api/v1/images/models/author/model/endpoints"
        }]
    })
    .to_string()
}

fn image_endpoints(resolution: &str, output_formats: &[&str]) -> String {
    serde_json::json!({
        "id": "author/model",
        "endpoints": [{
            "provider_name": "Provider",
            "provider_slug": "provider",
            "provider_tag": "provider-tag",
            "supported_parameters": {
                "resolution": {"type": "enum", "values": [resolution]},
                "aspect_ratio": {"type": "enum", "values": ["1:1"]},
                "n": {"type": "range", "min": 1, "max": 1},
                "output_format": {"type": "enum", "values": output_formats}
            },
            "allowed_passthrough_parameters": [],
            "supports_streaming": false,
            "pricing": [{"billable": "output_image", "unit": "image", "cost_usd": 0.0042}]
        }]
    })
    .to_string()
}

fn preview_png_base64() -> String {
    STANDARD.encode(preview_png_bytes())
}

fn preview_png_bytes() -> Vec<u8> {
    let image = image::DynamicImage::new_rgba8(2, 2);
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn image_request() -> crate::models::image_generation::GenerateImageRequest {
    crate::models::image_generation::GenerateImageRequest {
        prompt: "A quiet workspace".into(),
        model_id: "author/model".into(),
        resolution: "1024x1024".into(),
        aspect_ratio: "1:1".into(),
    }
}

#[tokio::test]
async fn image_generation_rechecks_exact_endpoint_pins_provider_and_posts_once() {
    let image = preview_png_base64();
    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": image, "media_type": "image/png"}],
        "usage": {"cost": 0.0042}
    })
    .to_string();
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let preview = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap();
    handle.join().unwrap();

    assert_eq!(
        preview.cost,
        crate::models::image_generation::ImageGenerationCost::ExactUsd {
            usd: "0.0042".into()
        }
    );
    assert!(preview.prompt_leaves_device);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/api/v1/images/models");
    assert_eq!(
        requests[1].path,
        "/api/v1/images/models/author/model/endpoints"
    );
    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, "/api/v1/images");
    assert!(requests
        .iter()
        .all(|request| request.authorization.as_deref() == Some("Bearer secret")));
    let body: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    assert_eq!(body["model"], "author/model");
    assert_eq!(body["prompt"], "A quiet workspace");
    assert_eq!(body["resolution"], "1024x1024");
    assert_eq!(body["aspect_ratio"], "1:1");
    assert_eq!(body["n"], 1);
    assert_eq!(body["stream"], false);
    assert_eq!(body["output_format"], "png");
    assert_eq!(
        body["provider"]["only"],
        serde_json::json!(["provider-tag"])
    );
    assert_eq!(body["provider"]["allow_fallbacks"], false);
    assert!(body["provider"].get("require_parameters").is_none());
}

#[tokio::test]
async fn image_generation_omits_output_format_when_endpoint_does_not_advertise_it() {
    let mut endpoints =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap();
    endpoints["endpoints"][0]["supported_parameters"]
        .as_object_mut()
        .unwrap()
        .remove("output_format");
    endpoints["endpoints"][0]["supported_parameters"]["input_references"] =
        serde_json::json!({"type": "range", "min": 0, "max": 4});
    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": preview_png_base64(), "media_type": "image/png"}]
    })
    .to_string();
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, endpoints.to_string()),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let preview = service
        .generate_image(image_request(), test_vault_scope())
        .await;
    handle.join().unwrap();
    let preview = preview.unwrap();

    assert_eq!(preview.media_type, "image/png");
    assert_eq!((preview.width, preview.height), (2, 2));
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let body: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    assert!(body.get("output_format").is_none());
}

#[tokio::test]
async fn image_generation_rejects_reference_required_endpoint_before_post() {
    let mut endpoints =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap();
    endpoints["endpoints"][0]["supported_parameters"]["input_references"] =
        serde_json::json!({"type": "range", "min": 1, "max": 4});
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, endpoints.to_string()),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();

    assert!(error.to_string().contains("input references"));
    assert_eq!(
        observed
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        0
    );
}

#[tokio::test]
async fn image_discovery_and_generation_require_text_input_capability() {
    let mut catalog =
        serde_json::from_str::<serde_json::Value>(&image_catalog("1024x1024", &["png"])).unwrap();
    let mut image_only = catalog["data"][0].clone();
    image_only["id"] = serde_json::json!("author/image-only");
    image_only["name"] = serde_json::json!("Image-reference-only model");
    image_only["architecture"]["input_modalities"] = serde_json::json!(["image"]);
    image_only["endpoints"] =
        serde_json::json!("/api/v1/images/models/author/image-only/endpoints");
    catalog["data"].as_array_mut().unwrap().push(image_only);
    let (base_url, _observed, handle) = spawn_image_server(vec![(200, catalog.to_string())]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let models = service.discover_image_models().await.unwrap();
    handle.join().unwrap();

    assert_eq!(
        models
            .iter()
            .map(|model| model.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["author/model"]
    );

    let mut image_only_catalog =
        serde_json::from_str::<serde_json::Value>(&image_catalog("1024x1024", &["png"])).unwrap();
    image_only_catalog["data"][0]["architecture"]["input_modalities"] =
        serde_json::json!(["image"]);
    let (base_url, observed, handle) =
        spawn_image_server(vec![(200, image_only_catalog.to_string())]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();

    assert!(error.to_string().contains("text input"));
    assert_eq!(observed.lock().unwrap().len(), 1);
}

#[test]
fn image_capability_parameters_accept_size_and_bounded_future_entries_only() {
    let mut catalog_value =
        serde_json::from_str::<serde_json::Value>(&image_catalog("1024x1024", &["png"])).unwrap();
    let catalog_parameters = catalog_value["data"][0]["supported_parameters"]
        .as_object_mut()
        .unwrap();
    catalog_parameters.insert(
        "size".into(),
        serde_json::json!({"type": "enum", "values": ["1024x1024"]}),
    );
    catalog_parameters.insert(
        "future_detail".into(),
        serde_json::json!({
            "type": "structured_choice_v2",
            "options": [{"id": "cinematic", "weight": 1}]
        }),
    );
    catalog_parameters["resolution"]["default"] = serde_json::json!("1024x1024");
    let catalog: ImageCatalogResponse = serde_json::from_value(catalog_value.clone()).unwrap();
    validate_image_catalog(&catalog).unwrap();

    let mut endpoints_value =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap();
    let endpoint_parameters = endpoints_value["endpoints"][0]["supported_parameters"]
        .as_object_mut()
        .unwrap();
    endpoint_parameters.insert(
        "size".into(),
        serde_json::json!({"type": "enum", "values": ["1024x1024"]}),
    );
    endpoint_parameters.insert(
        "future_detail".into(),
        serde_json::json!({
            "type": "structured_choice_v2",
            "options": [{"id": "cinematic", "weight": 1}]
        }),
    );
    endpoint_parameters["n"]["default"] = serde_json::json!(1);
    let endpoints: ImageEndpointsResponse =
        serde_json::from_value(endpoints_value.clone()).unwrap();
    validate_image_endpoints(&endpoints, "author/model").unwrap();

    let mut too_many_future_parameters =
        serde_json::from_str::<serde_json::Value>(&image_catalog("1024x1024", &["png"])).unwrap();
    let parameters = too_many_future_parameters["data"][0]["supported_parameters"]
        .as_object_mut()
        .unwrap();
    for index in 0..65 {
        parameters.insert(
            format!("future_capability_{index}"),
            serde_json::json!({"type": "boolean"}),
        );
    }
    let too_many: ImageCatalogResponse =
        serde_json::from_value(too_many_future_parameters).unwrap();
    assert!(validate_image_catalog(&too_many).is_err());

    let mut catalog_with_unknown_outer = catalog_value;
    catalog_with_unknown_outer["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ImageCatalogResponse>(catalog_with_unknown_outer).is_err());
    let mut endpoints_with_unknown_outer = endpoints_value;
    endpoints_with_unknown_outer["unexpected"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<ImageEndpointsResponse>(endpoints_with_unknown_outer).is_err()
    );
}

#[tokio::test]
async fn image_generation_requires_key_and_stops_before_post_on_capability_mismatch() {
    let unconfigured =
        OpenRouterService::new_for_image_tests(String::new(), "http://127.0.0.1:9/api/v1".into());
    assert!(unconfigured
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err()
        .to_string()
        .contains("not configured"));

    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("2048x2048", &["png"])),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("does not advertise"));
    assert_eq!(
        observed.lock().unwrap().len(),
        2,
        "mismatch must make zero POSTs"
    );
}

#[tokio::test]
async fn image_generation_surfaces_provider_code_without_retry() {
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
        (
            402,
            serde_json::json!({
                "error": {"message": "insufficient credits", "code": 402}
            })
            .to_string(),
        ),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();

    assert!(error.to_string().contains("insufficient credits"));
    assert!(error.to_string().contains("402"));
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn image_generation_requires_exactly_one_canonical_raster_item() {
    for data in [
        serde_json::json!([]),
        serde_json::json!([
            {"b64_json": preview_png_base64(), "media_type": "image/png"},
            {"b64_json": preview_png_base64(), "media_type": "image/png"}
        ]),
    ] {
        let generation = serde_json::json!({
            "created": 1_800_000_001u64,
            "data": data
        })
        .to_string();
        let (base_url, observed, handle) = spawn_image_server(vec![
            (200, image_catalog("1024x1024", &["png"])),
            (200, image_endpoints("1024x1024", &["png"])),
            (200, generation),
        ]);
        let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
        let error = service
            .generate_image(image_request(), test_vault_scope())
            .await
            .unwrap_err();
        handle.join().unwrap();
        assert!(error.to_string().contains("exactly one image"));
        assert_eq!(observed.lock().unwrap().len(), 3);
    }

    let mut noncanonical = preview_png_base64();
    noncanonical.pop();
    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": noncanonical, "media_type": "image/png"}]
    })
    .to_string();
    let (base_url, _observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("canonical standard base64"));
}

#[tokio::test]
async fn image_generation_infers_missing_mime_and_marks_missing_cost_unavailable() {
    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": preview_png_base64()}]
    })
    .to_string();
    let (base_url, _observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let preview = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap();
    handle.join().unwrap();

    assert_eq!(preview.media_type, "image/png");
    assert_eq!(preview.cost, ImageGenerationCost::Unavailable);
}

#[tokio::test]
async fn image_generation_rejects_mime_mismatch_and_unknown_response_fields() {
    for generation in [
        serde_json::json!({
            "created": 1_800_000_001u64,
            "data": [{"b64_json": preview_png_base64(), "media_type": "image/jpeg"}]
        }),
        serde_json::json!({
            "created": 1_800_000_001u64,
            "data": [{"b64_json": preview_png_base64(), "media_type": "image/png", "url": "https://example.invalid/image"}]
        }),
    ] {
        let (base_url, _observed, handle) = spawn_image_server(vec![
            (200, image_catalog("1024x1024", &["png"])),
            (200, image_endpoints("1024x1024", &["png"])),
            (200, generation.to_string()),
        ]);
        let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
        assert!(service
            .generate_image(image_request(), test_vault_scope())
            .await
            .is_err());
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn image_generation_rejects_rasterless_endpoint_before_post() {
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["svg"])),
        (200, image_endpoints("1024x1024", &["svg"])),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();

    assert!(error.to_string().contains("raster format"));
    assert_eq!(observed.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn image_generation_skips_null_tag_and_buffers_streaming_capable_endpoint() {
    let endpoints = serde_json::json!({
        "id": "author/model",
        "endpoints": [
            {
                "provider_name": "Unpinnable",
                "provider_slug": "unpinnable",
                "provider_tag": null,
                "supported_parameters": {
                    "resolution": {"type": "enum", "values": ["1024x1024"]},
                    "aspect_ratio": {"type": "enum", "values": ["1:1"]},
                    "n": {"type": "range", "min": 1, "max": 1},
                    "output_format": {"type": "enum", "values": ["png"]}
                },
                "allowed_passthrough_parameters": [],
                "supports_streaming": false,
                "pricing": []
            },
            {
                "provider_name": "Buffered Provider",
                "provider_slug": "buffered-provider",
                "provider_tag": "buffered-tag",
                "supported_parameters": {
                    "resolution": {"type": "enum", "values": ["1024x1024"]},
                    "aspect_ratio": {"type": "enum", "values": ["1:1"]},
                    "n": {"type": "range", "min": 1, "max": 1},
                    "output_format": {"type": "enum", "values": ["png"]}
                },
                "allowed_passthrough_parameters": ["seed"],
                "supports_streaming": true,
                "pricing": [{
                    "billable": "output_image",
                    "unit": "image",
                    "cost_usd": 0.0042,
                    "variant": "quality"
                }]
            }
        ]
    })
    .to_string();
    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": preview_png_base64(), "media_type": "image/png"}],
        "usage": {
            "prompt_tokens": 11,
            "completion_tokens": 22,
            "total_tokens": 33,
            "cost": 0.0042
        }
    })
    .to_string();
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, endpoints),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap();
    handle.join().unwrap();

    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let body: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    assert_eq!(body["stream"], false);
    assert_eq!(
        body["provider"]["only"],
        serde_json::json!(["buffered-tag"])
    );
}

#[tokio::test]
async fn image_generation_rejects_duplicate_usable_provider_tags_before_post() {
    let endpoint =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap()
            ["endpoints"][0]
            .clone();
    let endpoints = serde_json::json!({
        "id": "author/model",
        "endpoints": [endpoint.clone(), endpoint]
    })
    .to_string();
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, endpoints),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();

    assert!(error.to_string().contains("duplicate provider tag"));
    assert_eq!(observed.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn image_generation_omits_n_when_endpoint_uses_single_image_default() {
    let mut endpoints =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap();
    endpoints["endpoints"][0]["supported_parameters"]
        .as_object_mut()
        .unwrap()
        .remove("n");
    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": preview_png_base64(), "media_type": "image/png"}]
    })
    .to_string();
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, endpoints.to_string()),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);

    service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap();
    handle.join().unwrap();

    let requests = observed.lock().unwrap();
    let body: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    assert!(body.get("n").is_none());
}

#[tokio::test]
async fn image_discovery_rejects_duplicate_models_and_ambiguous_prices() {
    let mut duplicate_catalog =
        serde_json::from_str::<serde_json::Value>(&image_catalog("1024x1024", &["png"])).unwrap();
    let duplicate = duplicate_catalog["data"][0].clone();
    duplicate_catalog["data"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    let (base_url, _observed, handle) =
        spawn_image_server(vec![(200, duplicate_catalog.to_string())]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service.discover_image_models().await.unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("duplicate model ID"));

    let mut endpoints =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap();
    let duplicate_price = endpoints["endpoints"][0]["pricing"][0].clone();
    endpoints["endpoints"][0]["pricing"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_price);
    let (base_url, _observed, handle) = spawn_image_server(vec![(200, endpoints.to_string())]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let capability = service
        .image_model_capability("author/model")
        .await
        .unwrap();
    handle.join().unwrap();
    assert_eq!(capability.endpoints[0].published_price, None);
}

#[tokio::test]
async fn image_generation_rejects_negative_costs_and_bounds_provider_errors() {
    let mut endpoints =
        serde_json::from_str::<serde_json::Value>(&image_endpoints("1024x1024", &["png"])).unwrap();
    endpoints["endpoints"][0]["pricing"][0]["cost_usd"] = serde_json::json!(-0.1);
    let (base_url, observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, endpoints.to_string()),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("non-negative"));
    assert_eq!(observed.lock().unwrap().len(), 2);

    let generation = serde_json::json!({
        "created": 1_800_000_001u64,
        "data": [{"b64_json": preview_png_base64(), "media_type": "image/png"}],
        "usage": {"cost": -1}
    })
    .to_string();
    let (base_url, _observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
        (200, generation),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("non-negative"));

    let long_message = "private-provider-detail".repeat(300);
    let long_code = "secret-code".repeat(30);
    let (base_url, _observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
        (
            500,
            serde_json::json!({"error": {"message": long_message, "code": long_code}}).to_string(),
        ),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service
        .generate_image(image_request(), test_vault_scope())
        .await
        .unwrap_err();
    handle.join().unwrap();
    let error = error.to_string();
    assert!(error.contains("OpenRouter image request failed"));
    assert!(!error.contains("private-provider-detail"));
    assert!(!error.contains("secret-code"));
}

#[tokio::test]
async fn image_discovery_and_capability_are_bounded_strict_live_shapes() {
    let (base_url, _observed, handle) = spawn_image_server(vec![
        (200, image_catalog("1024x1024", &["png"])),
        (200, image_endpoints("1024x1024", &["png"])),
    ]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let models = service.discover_image_models().await.unwrap();
    let capability = service
        .image_model_capability("author/model")
        .await
        .unwrap();
    handle.join().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "author/model");
    assert_eq!(capability.endpoints[0].endpoint_id, "provider-tag");
    assert_eq!(
        capability.endpoints[0].published_price.as_deref(),
        Some("0.0042")
    );
    assert!(!capability.pricing_is_final);

    let oversized = " ".repeat(MAX_IMAGE_DISCOVERY_BYTES + 1);
    let (base_url, _observed, handle) = spawn_image_server(vec![(200, oversized)]);
    let service = OpenRouterService::new_for_image_tests("secret".into(), base_url);
    let error = service.discover_image_models().await.unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("byte limit"));
}

#[test]
fn image_receipts_are_bounded_leased_retriable_then_one_shot() {
    let receipt = GeneratedImageReceipt {
        bytes: preview_png_bytes(),
        media_type: "image/png".into(),
        width: 2,
        height: 2,
        prompt: "A quiet workspace".into(),
        model_id: "author/model".into(),
        resolution: "1024x1024".into(),
        aspect_ratio: "1:1".into(),
        vault_scope: test_vault_scope(),
        gateway: "openrouter".into(),
        provider_tag: "provider-tag".into(),
    };
    let service =
        OpenRouterService::new_for_image_tests("secret".into(), "http://127.0.0.1:9/api/v1".into());
    let receipt_id = service.insert_generated_image_for_tests(receipt).unwrap();

    assert!(service.lease_generated_image(&receipt_id).is_ok());
    assert!(service.lease_generated_image(&receipt_id).is_err());
    service.release_generated_image(&receipt_id);
    assert!(service.lease_generated_image(&receipt_id).is_ok());
    service.consume_generated_image(&receipt_id);
    assert!(service.lease_generated_image(&receipt_id).is_err());
}

#[tokio::test]
async fn image_receipt_expiry_is_triggered_without_a_later_store_operation() {
    let receipt = GeneratedImageReceipt {
        bytes: preview_png_bytes(),
        media_type: "image/png".into(),
        width: 2,
        height: 2,
        prompt: "A short-lived private prompt".into(),
        model_id: "author/model".into(),
        resolution: "1024x1024".into(),
        aspect_ratio: "1:1".into(),
        vault_scope: test_vault_scope(),
        gateway: "openrouter".into(),
        provider_tag: "provider-tag".into(),
    };
    let service =
        OpenRouterService::new_for_image_tests("secret".into(), "http://127.0.0.1:9/api/v1".into());
    let receipt_id = service
        .insert_generated_image_for_tests_with_ttl(receipt, Duration::from_millis(10))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(40)).await;

    let store = service.image_receipts.lock().unwrap();
    assert!(!store.entries.contains_key(&receipt_id));
    assert_eq!(store.total_bytes, 0);
}
