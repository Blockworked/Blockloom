//! The emulator's own screen service, the way Android Studio's embedded
//! emulator view talks to it: the emulator scales each frame itself and
//! pushes it when it changes, and touches go back as real pointer events.
//!
//! Only the handful of messages used are declared here, by hand, with the
//! field numbers of the emulator's `emulator_controller.proto`; prost skips
//! the fields this file doesn't name.

use super::{Context, Event, Frame, Phase, TouchSink, frames};
use image::RgbImage;
use std::time::Duration;
use tokio::sync::mpsc;
use tonic::client::Grpc;
use tonic::codegen::http::uri::PathAndQuery;
use tonic::transport::{Channel, Endpoint};
use tonic_prost::ProstCodec;

/// `ImageFormat.ImgFormat`.
const RGB888: i32 = 2;
/// `Touch.EventExpiration.NEVER_EXPIRE`: a finger held still stays down.
const NEVER_EXPIRE: i32 = 1;
/// A finger's pressure while down; zero is how a lift is spelled.
const PRESSED: i32 = 1024;

#[derive(Clone, PartialEq, prost::Message)]
pub struct ImageFormat {
    #[prost(int32, tag = "1")]
    pub format: i32,
    #[prost(uint32, tag = "3")]
    pub width: u32,
    #[prost(uint32, tag = "4")]
    pub height: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Image {
    #[prost(message, optional, tag = "1")]
    pub format: Option<ImageFormat>,
    #[prost(bytes = "vec", tag = "4")]
    pub image: Vec<u8>,
    #[prost(uint32, tag = "5")]
    pub seq: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Touch {
    #[prost(int32, tag = "1")]
    pub x: i32,
    #[prost(int32, tag = "2")]
    pub y: i32,
    #[prost(int32, tag = "3")]
    pub identifier: i32,
    #[prost(int32, tag = "4")]
    pub pressure: i32,
    #[prost(int32, tag = "7")]
    pub expiration: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TouchEvent {
    #[prost(message, repeated, tag = "1")]
    pub touches: Vec<Touch>,
}

/// `google.protobuf.Empty`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Empty {}

/// A frame as the emulator sent it, as plain pixels, or why it can't be.
pub(super) fn rgb_from(image: &Image, width: u32, height: u32) -> Result<RgbImage, String> {
    let (w, h) = image
        .format
        .as_ref()
        .filter(|f| f.width > 0 && f.height > 0)
        .map_or((width, height), |f| (f.width, f.height));
    let pixels = w as usize * h as usize;
    let rgb = if image.image.len() == pixels * 3 {
        RgbImage::from_raw(w, h, image.image.clone())
    } else if image.image.len() == pixels * 4 {
        // RGBA, if an emulator build answers that to an RGB request.
        let rgb = image
            .image
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2]])
            .collect();
        RgbImage::from_raw(w, h, rgb)
    } else {
        None
    };
    rgb.ok_or_else(|| {
        format!(
            "the emulator sent {} bytes for a {w}x{h} frame",
            image.image.len()
        )
    })
}

/// The size to ask frames at: `max_width` wide (never above the screen's
/// own width), in the screen's own proportions.
pub(super) fn frame_size(native: (u32, u32), max_width: u32) -> (u32, u32) {
    let width = max_width.min(native.0).max(1);
    let height = (u64::from(width) * u64::from(native.1) / u64::from(native.0.max(1))).max(1);
    (width, height as u32)
}

/// A pointer as the emulator spells it, in the screen's own pixels.
pub(super) fn touch_for(phase: Phase, x: f32, y: f32, native: (u32, u32)) -> TouchEvent {
    let at = |fraction: f32, extent: u32| {
        (fraction.clamp(0.0, 1.0) * extent.saturating_sub(1) as f32).round() as i32
    };
    TouchEvent {
        touches: vec![Touch {
            x: at(x, native.0),
            y: at(y, native.1),
            identifier: 0,
            pressure: if phase == Phase::Up { 0 } else { PRESSED },
            expiration: NEVER_EXPIRE,
        }],
    }
}

type Pointer = (Phase, f32, f32);

struct GrpcTouch(mpsc::UnboundedSender<Pointer>);

impl TouchSink for GrpcTouch {
    fn touch(&mut self, phase: Phase, x: f32, y: f32) {
        let _ = self.0.send((phase, x, y));
    }
}

/// Streams the emulator's screen until the session stops (Ok), or fails
/// before or during the stream, in which case the caller falls back to adb.
pub(super) fn run(ctx: &Context) -> Result<(), String> {
    let port = blockloom_core::android::emulator_grpc_port(&ctx.serial)
        .ok_or("no gRPC port known for this emulator")?;
    let native = blockloom_core::android::device_size(Some(&ctx.serial))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(stream(ctx, port, native))
}

async fn connect(port: u16) -> Result<Channel, String> {
    Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
        .map_err(|e| e.to_string())?
        .connect_timeout(Duration::from_millis(1500))
        .tcp_nodelay(true)
        .connect()
        .await
        .map_err(|e| format!("couldn't reach the emulator's gRPC port {port}: {e}"))
}

async fn stream(ctx: &Context, port: u16, native: (u32, u32)) -> Result<(), String> {
    let channel = connect(port).await?;
    let (width, height) = frame_size(native, ctx.max_width);
    let mut grpc = Grpc::new(channel.clone());
    grpc.ready().await.map_err(|e| e.to_string())?;
    let request = tonic::Request::new(ImageFormat {
        format: RGB888,
        width,
        height,
    });
    let path =
        PathAndQuery::from_static("/android.emulation.control.EmulatorController/streamScreenshot");
    let mut frames_in = grpc
        .server_streaming(request, path, ProstCodec::<ImageFormat, Image>::default())
        .await
        .map_err(|e| e.to_string())?
        .into_inner();

    ctx.size.set(native.0, native.1);
    let (touch_tx, touch_rx) = mpsc::unbounded_channel();
    let touches = tokio::spawn(send_touches(channel, touch_rx, native));
    let mut routed = false;
    let mut seq = 0u64;
    let result = loop {
        if ctx.stopped() {
            break Ok(());
        }
        // Wake now and then to notice the session ending.
        let next = match tokio::time::timeout(Duration::from_millis(200), frames_in.message()).await
        {
            Err(_) => continue,
            Ok(next) => next,
        };
        let image = match next {
            Ok(Some(image)) => image,
            Ok(None) => break Err("the emulator closed its screen stream".to_string()),
            Err(status) => break Err(status.to_string()),
        };
        let rgb = match rgb_from(&image, width, height) {
            Ok(rgb) => rgb,
            // A frame that doesn't add up means a service that isn't
            // speaking the protocol this was written against.
            Err(error) => break Err(error),
        };
        let packed = frames::pack_rgb(&rgb)?;
        if !routed {
            routed = true;
            if let Ok(mut routes) = ctx.touch.lock() {
                routes.grpc = Some(Box::new(GrpcTouch(touch_tx.clone())));
            }
        }
        seq += 1;
        (ctx.sink)(Event::Frame(Frame {
            serial: ctx.serial.clone(),
            image: packed.url,
            width: packed.width,
            height: packed.height,
            device_width: native.0,
            device_height: native.1,
            seq,
            transport: "grpc",
        }));
    };
    if let Ok(mut routes) = ctx.touch.lock() {
        routes.grpc = None;
    }
    touches.abort();
    result
}

/// Sends pointers to the emulator in order, skipping to the newest move
/// when it falls behind.
async fn send_touches(
    channel: Channel,
    mut pointers: mpsc::UnboundedReceiver<Pointer>,
    native: (u32, u32),
) {
    let mut grpc = Grpc::new(channel);
    let path = PathAndQuery::from_static("/android.emulation.control.EmulatorController/sendTouch");
    let mut pending: Option<Pointer> = None;
    loop {
        let (phase, x, y) = match pending.take() {
            Some(pointer) => pointer,
            None => match pointers.recv().await {
                Some(pointer) => pointer,
                None => return,
            },
        };
        // Moves queued behind this one are stale: keep only the newest, but
        // never swallow a press or a lift.
        let mut latest = (phase, x, y);
        if phase == Phase::Move {
            while let Ok(next) = pointers.try_recv() {
                if next.0 == Phase::Move {
                    latest = next;
                } else {
                    pending = Some(next);
                    break;
                }
            }
        }
        if grpc.ready().await.is_err() {
            return;
        }
        let event = touch_for(latest.0, latest.1, latest.2, native);
        let sent = grpc
            .unary(
                tonic::Request::new(event),
                path.clone(),
                ProstCodec::<TouchEvent, Empty>::default(),
            )
            .await;
        if let Err(status) = sent {
            tracing::debug!("sendTouch: {status}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn frames_ask_for_the_screens_proportions() {
        assert_eq!(frame_size((1080, 2400), 540), (540, 1200));
        // Never above the screen's own width.
        assert_eq!(frame_size((720, 1600), 1080), (720, 1600));
        assert_eq!(frame_size((1080, 2400), 144), (144, 320));
    }

    #[test]
    fn pointers_become_native_pixels_and_lifts_drop_pressure() {
        let down = touch_for(Phase::Down, 0.5, 1.0, (1080, 2400));
        assert_eq!(down.touches[0].x, 540);
        assert_eq!(down.touches[0].y, 2399);
        assert_eq!(down.touches[0].pressure, PRESSED);
        let up = touch_for(Phase::Up, 2.0, -1.0, (1080, 2400));
        assert_eq!((up.touches[0].x, up.touches[0].y), (1079, 0));
        assert_eq!(up.touches[0].pressure, 0);
    }

    #[test]
    fn rgb_and_rgba_frames_decode_and_odd_ones_are_refused() {
        let frame = |bytes: usize| Image {
            format: Some(ImageFormat {
                format: RGB888,
                width: 2,
                height: 2,
            }),
            image: vec![9; bytes],
            seq: 1,
        };
        assert_eq!(rgb_from(&frame(12), 2, 2).unwrap().dimensions(), (2, 2));
        assert_eq!(rgb_from(&frame(16), 2, 2).unwrap().dimensions(), (2, 2));
        assert!(rgb_from(&frame(13), 2, 2).is_err());
    }

    #[test]
    fn the_messages_keep_the_emulators_field_numbers() {
        // ImageFormat { format: RGB888 (2), width: 3, height: 4 }.
        let bytes = ImageFormat {
            format: RGB888,
            width: 300,
            height: 4,
        }
        .encode_to_vec();
        assert_eq!(bytes, [0x08, 0x02, 0x18, 0xac, 0x02, 0x20, 0x04]);
        // A Touch's pressure is field 4 and its expiration field 7.
        let touch = Touch {
            x: 1,
            y: 2,
            identifier: 0,
            pressure: 1024,
            expiration: NEVER_EXPIRE,
        }
        .encode_to_vec();
        assert_eq!(
            touch,
            [0x08, 0x01, 0x10, 0x02, 0x20, 0x80, 0x08, 0x38, 0x01]
        );
        // A reply with fields this file doesn't name still decodes.
        let image = Image::decode(&[0x4a, 0x00, 0x28, 0x07][..]).unwrap();
        assert_eq!(image.seq, 7);
    }
}

#[cfg(test)]
#[cfg(unix)]
mod service_tests {
    use super::super::testing::{context, temp};
    use super::*;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::task::Poll;
    use tokio_stream::{Stream, StreamExt};
    use tonic::codegen::{Body, BoxFuture, Service, StdError, http};
    use tonic::server::{NamedService, ServerStreamingService, UnaryService};

    /// The emulator's service as far as this client uses it: a screen that
    /// streams two frames, and a record of what was asked and touched.
    #[derive(Clone, Default)]
    struct Emulator {
        asked: Arc<Mutex<Vec<ImageFormat>>>,
        touched: Arc<Mutex<Vec<TouchEvent>>>,
    }

    impl NamedService for Emulator {
        const NAME: &'static str = "android.emulation.control.EmulatorController";
    }

    struct Screen(Emulator);
    impl ServerStreamingService<ImageFormat> for Screen {
        type Response = Image;
        type ResponseStream = Pin<Box<dyn Stream<Item = Result<Image, tonic::Status>> + Send>>;
        type Future =
            std::future::Ready<Result<tonic::Response<Self::ResponseStream>, tonic::Status>>;

        fn call(&mut self, request: tonic::Request<ImageFormat>) -> Self::Future {
            let asked = request.into_inner();
            self.0.asked.lock().unwrap().push(asked.clone());
            let frame = |shade: u8| Image {
                format: Some(asked.clone()),
                image: vec![shade; asked.width as usize * asked.height as usize * 3],
                seq: 1,
            };
            // Two frames, then the stream stays open like a live screen.
            let stream =
                tokio_stream::iter([Ok(frame(30)), Ok(frame(200))]).chain(tokio_stream::pending());
            std::future::ready(Ok(tonic::Response::new(Box::pin(stream))))
        }
    }

    struct Touches(Emulator);
    impl UnaryService<TouchEvent> for Touches {
        type Response = Empty;
        type Future = std::future::Ready<Result<tonic::Response<Empty>, tonic::Status>>;

        fn call(&mut self, request: tonic::Request<TouchEvent>) -> Self::Future {
            self.0.touched.lock().unwrap().push(request.into_inner());
            std::future::ready(Ok(tonic::Response::new(Empty {})))
        }
    }

    impl<B> Service<http::Request<B>> for Emulator
    where
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::Body>;
        type Error = Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;

        fn poll_ready(&mut self, _: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, request: http::Request<B>) -> Self::Future {
            let this = self.clone();
            Box::pin(async move {
                Ok(match request.uri().path() {
                    "/android.emulation.control.EmulatorController/streamScreenshot" => {
                        let mut grpc =
                            tonic::server::Grpc::new(ProstCodec::<Image, ImageFormat>::default());
                        grpc.server_streaming(Screen(this), request).await
                    }
                    "/android.emulation.control.EmulatorController/sendTouch" => {
                        let mut grpc =
                            tonic::server::Grpc::new(ProstCodec::<Empty, TouchEvent>::default());
                        grpc.unary(Touches(this), request).await
                    }
                    _ => tonic::Status::unimplemented("not in this test").into_http(),
                })
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_emulators_screen_streams_and_touches_go_back_to_it() {
        let emulator = Emulator::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let service = emulator.clone();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
        });

        let dir = temp("grpc");
        let (ctx, frames) = context(dir.join("adb"));
        let ctx = ctx;
        let (stop, touch) = (ctx.stop.clone(), ctx.touch.clone());
        // Touch once the stream is up, and stop once the emulator has heard.
        let (heard, seen) = (emulator.touched.clone(), frames.clone());
        tokio::spawn(async move {
            let mut touched = false;
            for _ in 0..500 {
                if seen.lock().unwrap().len() >= 2 {
                    if !touched {
                        touched = true;
                        let mut routes = touch.lock().unwrap();
                        let route = routes.grpc.as_mut().expect("the emulator's route is up");
                        route.touch(Phase::Down, 0.5, 0.5);
                        route.touch(Phase::Up, 0.5, 0.5);
                    }
                    if heard.lock().unwrap().len() >= 2 {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            stop.store(true, Ordering::Relaxed);
        });

        stream(&ctx, port, (16, 32)).await.unwrap();

        // Frames were asked at the panel's width in the screen's proportions.
        let asked = emulator.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(
            (asked[0].format, asked[0].width, asked[0].height),
            (RGB888, 8, 16)
        );
        let frames = frames.lock().unwrap();
        assert!(frames.len() >= 2, "{frames:?}");
        assert_eq!(frames[0].transport, "grpc");
        assert_eq!((frames[0].width, frames[0].height), (8, 16));
        assert_eq!((frames[0].device_width, frames[0].device_height), (16, 32));
        assert_ne!(frames[0].image, frames[1].image);
        // Pointers arrived in the screen's own pixels, down then up.
        let touched = emulator.touched.lock().unwrap();
        assert_eq!(touched.len(), 2, "{touched:?}");
        assert_eq!(touched[0].touches[0].pressure, PRESSED);
        assert_eq!((touched[0].touches[0].x, touched[0].touches[0].y), (8, 16));
        assert_eq!(touched[1].touches[0].pressure, 0);
        // Once the stream is over its route is gone.
        assert!(ctx.touch.lock().unwrap().grpc.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nothing_listening_is_an_error_the_caller_falls_back_from() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (ctx, _) = context(std::path::PathBuf::from("adb"));
        let error = stream(&ctx, port, (16, 32)).await.unwrap_err();
        assert!(error.contains("couldn't reach"), "{error}");
    }
}
