// SPDX-License-Identifier: MIT OR Apache-2.0

//! The IOPaint client against a fake server (wiremock on 127.0.0.1): readiness, request bodies,
//! PNG round trips, outpaint geometry, plugin masks, and the `{detail}` error shape. Offline.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use redrob_ai::{
    AiError, Expand, Mask, Rgba8Image,
    iopaint::{Client, InpaintOptions},
    pixels::png_from_data_url,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

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
