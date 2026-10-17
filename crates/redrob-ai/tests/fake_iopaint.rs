// SPDX-License-Identifier: MIT OR Apache-2.0

//! The IOPaint client against a fake server (wiremock on 127.0.0.1): readiness, request bodies,
//! PNG round trips, the region of interest, the raw-file transport, outpaint tiling, plugin
//! masks, and the `{detail}` error shape. Offline; every image is built in memory.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use redrob_ai::{
    AiError, Expand, Mask, Rect, Rgba8Image,
    iopaint::{
        Client, InpaintOptions, Transport,
        transport::{FileExchange, TransportMode},
    },
    pixels::png_from_data_url,
    region,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

/// Answers an inpaint the way the server would: a solid `fill` image of the size that was sent
/// (base64 body), so a test can check what crossed the wire from the result alone.
struct EchoSize {
    fill: [u8; 4],
}

impl Respond for EchoSize {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = body_of(request);
        let sent =
            Rgba8Image::from_png(&png_from_data_url(body["image"].as_str().unwrap()).unwrap())
                .unwrap();
        let out = Rgba8Image::filled(sent.width, sent.height, self.fill).unwrap();
        png_response(out.to_png().unwrap())
    }
}

/// The shim's file routes: reads the raw input the client wrote, writes a raw result next to it
/// (RGB for inpaint, RGBA for a plugin image, L8 for a plugin mask), answers with its geometry.
struct FileRoutes;

impl Respond for FileRoutes {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = body_of(request);
        let width = body["width"].as_u64().unwrap() as u32;
        let height = body["height"].as_u64().unwrap() as u32;
        let out_path = body["output_path"].as_str().unwrap();
        let image_bytes = std::fs::read(body["image_path"].as_str().unwrap()).unwrap();
        let pixels = (width * height) as usize;
        let channels = match request.url.path() {
            "/api/v1/redrob/inpaint" => {
                assert_eq!(image_bytes.len(), pixels * 3, "RGB in");
                let mask = std::fs::read(body["mask_path"].as_str().unwrap()).unwrap();
                assert_eq!(mask.len(), pixels, "L8 mask in");
                // Invert the picture: easy to check, and no pixel is left as it came.
                let out: Vec<u8> = image_bytes.iter().map(|b| 255 - b).collect();
                std::fs::write(out_path, out).unwrap();
                3
            }
            "/api/v1/redrob/plugin_gen_image" => {
                assert_eq!(image_bytes.len(), pixels * 4, "RGBA in");
                let scale = body["scale"].as_f64().unwrap() as u32;
                let out = Rgba8Image::filled(width * scale, height * scale, [9, 8, 7, 255])
                    .unwrap();
                std::fs::write(out_path, &out.data).unwrap();
                return ResponseTemplate::new(200).set_body_json(json!({
                    "width": width * scale, "height": height * scale, "channels": 4
                }));
            }
            "/api/v1/redrob/plugin_gen_mask" => {
                assert_eq!(image_bytes.len(), pixels * 4, "RGBA in");
                let mut mask = vec![0u8; pixels];
                mask[0] = 255;
                std::fs::write(out_path, mask).unwrap();
                1
            }
            other => panic!("unexpected route {other}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "width": width, "height": height, "channels": channels
        }))
    }
}

fn checker(w: u32, h: u32) -> Rgba8Image {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(if (x + y) % 2 == 0 {
                &[200, 30, 30, 255]
            } else {
                &[30, 30, 200, 0]
            });
        }
    }
    Rgba8Image::new(w, h, data).unwrap()
}

fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

fn png_response(png: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "image/png")
        .set_body_bytes(png)
}

#[tokio::test]
async fn server_config_readiness_and_parse() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/server-config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "modelInfos": [{"name": "lama", "need_prompt": false}],
            "plugins": [{"name": "RemoveBG"}],
            "removeBGModel": "u2net", "removeBGModels": ["u2net"],
            "samplers": ["ddim"]
        })))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let config = client.server_config().await.unwrap();
    assert!(config.has_model("lama"));
    assert!(config.has_plugin("RemoveBG"));
    assert_eq!(config.plugin_models["RemoveBG"].current, "u2net");
    assert_eq!(config.samplers, vec!["ddim"]);

    // Not up yet: a connection refusal is a transport error, not a server one.
    let dead = Client::for_port(1).unwrap();
    assert!(matches!(dead.server_config().await, Err(AiError::Http(_))));
}

#[tokio::test]
async fn inpaint_sends_opaque_image_mask_and_options_and_clips_the_result() {
    let server = MockServer::start().await;
    let image = checker(4, 4);
    // The server echoes a solid green image of the same size.
    let green = Rgba8Image::filled(4, 4, [0, 255, 0, 255]).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(png_response(green.to_png().unwrap()))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let mask = Mask::new(4, 4, {
        let mut m = vec![0u8; 16];
        m[5] = 255;
        m[6] = 255;
        m
    })
    .unwrap();
    let options = InpaintOptions {
        prompt: Some("a bench".into()),
        negative_prompt: Some("blur".into()),
        sd_steps: Some(20),
        sd_strength: Some(1.0),
        sd_guidance_scale: Some(7.5),
        sd_seed: Some(-1),
        sd_sampler: Some("ddim".into()),
        sd_keep_unmasked_area: Some(true),
        sd_match_histograms: Some(false),
        sd_lcm_lora: Some(false),
        powerpaint_task: Some("text-guided".into()),
        enable_controlnet: Some(true),
        controlnet_method: Some("canny".into()),
        enable_brushnet: Some(false),
        brushnet_method: None,
        paint_by_example_image: None,
    };
    let result = client.inpaint(&image, &mask, &options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("content-type").unwrap(),
        "application/json"
    );
    let body = body_of(&requests[0]);
    for key in [
        "image",
        "mask",
        "prompt",
        "negative_prompt",
        "sd_steps",
        "sd_strength",
        "sd_guidance_scale",
        "sd_seed",
        "sd_sampler",
        "sd_keep_unmasked_area",
        "sd_match_histograms",
        "sd_lcm_lora",
        "powerpaint_task",
        "enable_controlnet",
        "controlnet_method",
        "enable_brushnet",
    ] {
        assert!(body.get(key).is_some(), "missing {key}");
    }
    assert!(body.get("brushnet_method").is_none());
    assert_eq!(body["sd_steps"], 20);
    assert_eq!(body["prompt"], "a bench");

    // The image went out opaque: the transparent blue checker cells are white.
    let sent =
        Rgba8Image::from_png(&png_from_data_url(body["image"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!((sent.width, sent.height), (4, 4));
    assert_eq!(sent.pixel(0, 0), [200, 30, 30, 255]);
    assert_eq!(sent.pixel(1, 0), [255, 255, 255, 255]);
    let sent_mask =
        Mask::from_png(&png_from_data_url(body["mask"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(sent_mask, mask);

    // The result is clipped to the mask: green only where the mask was set.
    assert_eq!((result.width, result.height), (4, 4));
    assert_eq!(result.pixel(1, 1), [0, 255, 0, 255]);
    assert_eq!(result.pixel(2, 1), [0, 255, 0, 255]);
    assert_eq!(result.pixel(0, 0)[3], 0);
    assert_eq!(result.pixel(3, 3)[3], 0);
}

#[tokio::test]
async fn inpaint_without_keep_unmasked_returns_the_whole_result() {
    let server = MockServer::start().await;
    let green = Rgba8Image::filled(2, 2, [0, 255, 0, 255]).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(png_response(green.to_png().unwrap()))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let image = checker(2, 2);
    let mask = Mask::new(2, 2, vec![255, 0, 0, 0]).unwrap();
    let options = InpaintOptions {
        sd_keep_unmasked_area: Some(false),
        ..InpaintOptions::default()
    };
    let result = client.inpaint(&image, &mask, &options).await.unwrap();
    assert_eq!(result, green);
    // A mask of the wrong size never reaches the server.
    let wrong = Mask::filled(3, 3, 255).unwrap();
    assert!(matches!(
        client.inpaint(&image, &wrong, &options).await,
        Err(AiError::Geometry(_))
    ));
}

#[tokio::test]
async fn outpaint_pads_masks_the_border_and_posts_inpaint() {
    let server = MockServer::start().await;
    let image = Rgba8Image::filled(4, 4, [10, 20, 30, 255]).unwrap();
    let expand = Expand {
        left: 1,
        top: 2,
        right: 3,
        bottom: 0,
    };
    let filled = Rgba8Image::filled(8, 6, [7, 7, 7, 255]).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(png_response(filled.to_png().unwrap()))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let options = InpaintOptions {
        prompt: Some("more sky".into()),
        ..InpaintOptions::default()
    };
    let result = client.outpaint(&image, expand, &options).await.unwrap();
    assert_eq!((result.width, result.height), (8, 6));
    // Border filled by the server, picture area transparent in the result layer.
    assert_eq!(result.pixel(0, 0), [7, 7, 7, 255]);
    assert_eq!(result.pixel(7, 5), [7, 7, 7, 255]);
    assert_eq!(result.pixel(1, 2)[3], 0);
    assert_eq!(result.pixel(4, 5)[3], 0);

    let requests = server.received_requests().await.unwrap();
    let body = body_of(&requests[0]);
    assert_eq!(body["prompt"], "more sky");
    let sent =
        Rgba8Image::from_png(&png_from_data_url(body["image"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!((sent.width, sent.height), (8, 6));
    assert_eq!(sent.pixel(0, 0), [0, 0, 0, 255]);
    assert_eq!(sent.pixel(1, 2), [10, 20, 30, 255]);
    let sent_mask =
        Mask::from_png(&png_from_data_url(body["mask"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(sent_mask.coverage(), 48 - 16);
    assert_eq!(sent_mask.get(0, 0), 255);
    assert_eq!(sent_mask.get(1, 2), 0);

    // A wrong-size answer is refused.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(png_response(image.to_png().unwrap()))
        .mount(&server)
        .await;
    assert!(matches!(
        client.outpaint(&image, expand, &options).await,
        Err(AiError::Geometry(_))
    ));
}

#[tokio::test]
async fn plugin_gen_mask_encodes_clicks_and_thresholds_alpha() {
    let server = MockServer::start().await;
    // IOPaint paints the selection with a translucent colour: alpha > 0 means selected.
    let painted = Rgba8Image::new(
        2,
        2,
        vec![255, 0, 0, 90, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 1],
    )
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/run_plugin_gen_mask"))
        .respond_with(png_response(painted.to_png().unwrap()))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let image = checker(2, 2);

    let mask = client
        .plugin_gen_mask("InteractiveSeg", &image, &[(1, 1, true), (0, 1, false)])
        .await
        .unwrap();
    assert_eq!(mask.data, vec![255, 0, 0, 255]);
    let subject = client
        .plugin_gen_mask("RemoveBG", &image, &[])
        .await
        .unwrap();
    assert_eq!(subject.coverage(), 2);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let seg = body_of(&requests[0]);
    assert_eq!(seg["name"], "InteractiveSeg");
    assert_eq!(seg["clicks"], json!([[1, 1, 1], [0, 1, 0]]));
    assert!(
        seg["image"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    let bg = body_of(&requests[1]);
    assert_eq!(bg["name"], "RemoveBG");
    assert!(bg.get("clicks").is_none());

    // InteractiveSeg needs at least one click; nothing is sent.
    assert!(matches!(
        client.plugin_gen_mask("InteractiveSeg", &image, &[]).await,
        Err(AiError::Geometry(_))
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn plugin_gen_image_sends_name_image_scale() {
    let server = MockServer::start().await;
    let big = Rgba8Image::filled(4, 4, [1, 2, 3, 255]).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/run_plugin_gen_image"))
        .respond_with(png_response(big.to_png().unwrap()))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let image = checker(2, 2);
    let result = client
        .plugin_gen_image("RealESRGAN", &image, 2.0)
        .await
        .unwrap();
    assert_eq!(result.uniform_scale_from(2, 2), Some(2.0));
    let body = body_of(&server.received_requests().await.unwrap()[0]);
    assert_eq!(body["name"], "RealESRGAN");
    assert_eq!(body["scale"], 2.0);
    assert!(
        body["image"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );

    // Out-of-range scale is clamped; an upscale past 64 Mi-pixels is refused before sending.
    client
        .plugin_gen_image("GFPGAN", &image, 9.0)
        .await
        .unwrap();
    let body = body_of(&server.received_requests().await.unwrap()[1]);
    assert_eq!(body["scale"], 4.0);
    let huge = Rgba8Image::filled(5000, 5000, [0; 4]).unwrap();
    assert!(matches!(
        client.plugin_gen_image("RealESRGAN", &huge, 4.0).await,
        Err(AiError::Geometry(_))
    ));
}

#[tokio::test]
async fn model_switches_post_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/model"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/switch_plugin_model"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    client.set_model("migan").await.unwrap();
    client
        .switch_plugin_model("RemoveBG", "isnet-general-use")
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(body_of(&requests[0]), json!({ "name": "migan" }));
    assert_eq!(
        body_of(&requests[1]),
        json!({ "plugin_name": "RemoveBG", "model_name": "isnet-general-use" })
    );
}

#[tokio::test]
async fn error_detail_becomes_a_typed_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(json!({ "detail": "CUDA out of memory" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/model"))
        .respond_with(
            ResponseTemplate::new(422).set_body_json(json!({ "errors": "no such model" })),
        )
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let image = checker(2, 2);
    let mask = Mask::filled(2, 2, 255).unwrap();
    match client
        .inpaint(&image, &mask, &InpaintOptions::default())
        .await
    {
        Err(AiError::Server { status, detail }) => {
            assert_eq!(status, 500);
            assert_eq!(detail, "CUDA out of memory");
        }
        other => panic!("{other:?}"),
    }
    match client.set_model("x").await {
        Err(AiError::Server { status, detail }) => {
            assert_eq!(status, 422);
            assert_eq!(detail, "no such model");
        }
        other => panic!("{other:?}"),
    }
    let shown = client.set_model("x").await.unwrap_err().to_string();
    assert_eq!(shown, "AI engine error: no such model");
}

#[tokio::test]
async fn a_non_png_200_is_a_png_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/run_plugin_gen_image"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not a png"))
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    assert!(matches!(
        client
            .plugin_gen_image("RemoveBG", &checker(2, 2), 1.0)
            .await,
        Err(AiError::Png(_))
    ));
}

#[tokio::test]
async fn inpaint_sends_only_the_region_around_the_mask_and_places_it_back() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(EchoSize {
            fill: [0, 255, 0, 255],
        })
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    let image = Rgba8Image::filled(2048, 1024, [40, 40, 40, 255]).unwrap();
    let mut mask = Mask::filled(2048, 1024, 0).unwrap();
    mask.fill_rect(Rect::new(1000, 500, 100, 80), 255);

    let (painted, (x, y)) = client
        .inpaint_region(&image, &mask, &InpaintOptions::default())
        .await
        .unwrap();
    let region = region::inpaint_region(&mask, region::ROI_MARGIN).unwrap();
    assert_eq!((x, y), (region.x, region.y));
    assert_eq!((painted.width, painted.height), (region.width, region.height));
    assert!(painted.width <= 100 + 2 * region::ROI_MARGIN + 64);
    // Green only inside the mask; the margin came back transparent.
    assert_eq!(painted.pixel(1000 - x, 500 - y), [0, 255, 0, 255]);
    assert_eq!(painted.pixel(0, 0)[3], 0);

    // The wire carried the region, not the 2K canvas.
    let requests = server.received_requests().await.unwrap();
    let body = body_of(&requests[0]);
    let sent =
        Rgba8Image::from_png(&png_from_data_url(body["image"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!((sent.width, sent.height), (region.width, region.height));

    // `inpaint` pastes the region into a canvas-sized transparent layer.
    let full = client
        .inpaint(&image, &mask, &InpaintOptions::default())
        .await
        .unwrap();
    assert_eq!((full.width, full.height), (2048, 1024));
    assert_eq!(full.pixel(1050, 540), [0, 255, 0, 255]);
    assert_eq!(full.pixel(0, 0)[3], 0);
    assert_eq!(full.pixel(2047, 1023)[3], 0);

    // A prompted fill takes more context; an empty selection never reaches the server.
    let prompted = InpaintOptions {
        prompt: Some("a bench".into()),
        ..InpaintOptions::default()
    };
    let (wide, _) = client.inpaint_region(&image, &mask, &prompted).await.unwrap();
    assert!(wide.width > painted.width);
    let before = server.received_requests().await.unwrap().len();
    assert!(matches!(
        client
            .inpaint(&image, &Mask::filled(2048, 1024, 0).unwrap(), &prompted)
            .await,
        Err(AiError::Geometry(_))
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn file_transport_writes_raw_files_sends_paths_and_cleans_up() {
    let server = MockServer::start().await;
    for route in [
        "/api/v1/redrob/inpaint",
        "/api/v1/redrob/plugin_gen_image",
        "/api/v1/redrob/plugin_gen_mask",
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(FileRoutes)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/api/v1/redrob/transport"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "file": true })))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let exchange_dir = tmp.path().join("exchange");
    let transport = Transport::choose(
        TransportMode::File,
        &server.uri(),
        Some(&exchange_dir),
        true,
    );
    assert!(transport.is_file(), "wiremock is on loopback");
    let client = Client::with_transport(server.uri(), transport).unwrap();
    assert!(client.has_file_transport().await);

    // Inpaint: a small picture goes whole; the shim inverts it.
    let image = checker(4, 4);
    let mask = Mask::new(4, 4, {
        let mut m = vec![0u8; 16];
        m[5] = 255;
        m
    })
    .unwrap();
    let result = client
        .inpaint(&image, &mask, &InpaintOptions::default())
        .await
        .unwrap();
    // (1, 1) is a red checker cell (200, 30, 30) over white -> inverted (55, 225, 225).
    assert_eq!(result.pixel(1, 1), [55, 225, 225, 255]);
    assert_eq!(result.pixel(0, 0)[3], 0);
    let requests = server.received_requests().await.unwrap();
    let body = body_of(&requests[1]);
    assert!(body.get("image").is_none(), "no base64 in the body");
    assert!(body["image_path"].as_str().unwrap().starts_with(exchange_dir.to_str().unwrap()));
    assert_eq!(body["width"], 4);
    assert_eq!(body["sd_keep_unmasked_area"], Value::Null);
    // The per-request files are gone once the result is read.
    assert_eq!(std::fs::read_dir(&exchange_dir).unwrap().count(), 0);

    // Plugin image: RGBA in, RGBA out at the plugin's size.
    let up = client.plugin_gen_image("RealESRGAN", &image, 2.0).await.unwrap();
    assert_eq!((up.width, up.height), (8, 8));
    assert_eq!(up.pixel(7, 7), [9, 8, 7, 255]);
    // Plugin mask: L8 out.
    let subject = client.plugin_gen_mask("RemoveBG", &image, &[]).await.unwrap();
    assert_eq!(subject.coverage(), 1);
    assert_eq!(subject.get(0, 0), 255);
    assert_eq!(std::fs::read_dir(&exchange_dir).unwrap().count(), 0);

    // A server error on the file route is the same typed error.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/redrob/inpaint"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "detail": "oom" })))
        .mount(&server)
        .await;
    match client.inpaint(&image, &mask, &InpaintOptions::default()).await {
        Err(AiError::Server { status, detail }) => {
            assert_eq!(status, 500);
            assert_eq!(detail, "oom");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read_dir(&exchange_dir).unwrap().count(), 0);
}

#[tokio::test]
async fn a_plain_server_has_no_file_transport() {
    let server = MockServer::start().await;
    let client = Client::new(server.uri()).unwrap();
    assert!(!client.has_file_transport().await);
    assert_eq!(client.transport().mode(), TransportMode::Base64);
}

#[tokio::test]
async fn outpaint_past_the_tile_size_walks_the_border_in_tiles() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/inpaint"))
        .respond_with(EchoSize {
            fill: [7, 7, 7, 255],
        })
        .mount(&server)
        .await;
    let client = Client::new(server.uri()).unwrap();
    // 1500 × 300 grown 600 px to the right and 100 px on top: 2100 × 400, wider than a tile.
    let image = Rgba8Image::filled(1500, 300, [10, 20, 30, 255]).unwrap();
    let expand = Expand {
        left: 0,
        top: 100,
        right: 600,
        bottom: 0,
    };
    let result = client
        .outpaint(&image, expand, &InpaintOptions::default())
        .await
        .unwrap();
    assert_eq!((result.width, result.height), (2100, 400));
    // Border painted, picture transparent.
    assert_eq!(result.pixel(2099, 399), [7, 7, 7, 255]);
    assert_eq!(result.pixel(1500, 200), [7, 7, 7, 255]);
    assert_eq!(result.pixel(0, 0), [7, 7, 7, 255]);
    assert_eq!(result.pixel(1000, 50), [7, 7, 7, 255]);
    assert_eq!(result.pixel(0, 100)[3], 0);
    assert_eq!(result.pixel(1499, 399)[3], 0);
    let requests = server.received_requests().await.unwrap();
    let expected = region::outpaint_tiles(Rect::new(0, 100, 1500, 300), 2100, 400);
    assert_eq!(requests.len(), expected.len());
    assert!(requests.len() >= 3, "{} tiles", requests.len());
    for (request, tile) in requests.iter().zip(&expected) {
        let body = body_of(request);
        let sent =
            Rgba8Image::from_png(&png_from_data_url(body["image"].as_str().unwrap()).unwrap())
                .unwrap();
        assert!(sent.width <= region::TILE && sent.height <= region::TILE);
        assert_eq!((sent.width, sent.height), (tile.rect.width, tile.rect.height));
        let sent_mask =
            Mask::from_png(&png_from_data_url(body["mask"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(sent_mask.coverage(), tile.paint.area() as usize);
    }
    // Every tile carried known context: never a mask covering the whole tile.
    for request in &requests {
        let body = body_of(request);
        let sent_mask =
            Mask::from_png(&png_from_data_url(body["mask"].as_str().unwrap()).unwrap()).unwrap();
        assert!(sent_mask.coverage() < sent_mask.data.len());
    }
}
