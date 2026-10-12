//! Windows screen capture with Windows.Graphics.Capture (Windows 10 1803+).
//!
//! [`WindowsGraphicsCaptureSource`] captures the primary monitor and implements
//! [`FrameSource`]. A dedicated thread owns every COM object: it waits for the
//! frame pool's `FrameArrived` signal, copies each frame from the GPU into
//! memory, scales it to the size the encoder wants, and sends it on.
//!
//! * **Timestamps** come from the frame's `SystemRelativeTime`, the same QPC
//!   clock WASAPI loopback uses, and are re-based on the shared video epoch so
//!   audio and video line up ([`SharedEpoch`]).
//! * **Pacing**: a 144 Hz monitor delivers more frames than the 60 fps the
//!   recording wants; [`Pacer`] keeps an even 60. A screen that doesn't change
//!   delivers nothing, as with ScreenCaptureKit on macOS.
//! * **Size**: the monitor's own pixel size is scaled to the configured one
//!   (see [`primary_display_size`] and [`super::scale::fit_even`]).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::mpsc;
use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use super::clock::SharedEpoch;
use super::pacer::Pacer;
use super::scale::scale_bgra;
use super::{CaptureError, CaptureStats, FrameSource, RawFrame};

/// Frames buffered between the capture thread and the encoder. Small on
/// purpose: at 60 fps, 8 frames are ~130 ms of slack before frames are dropped.
const CHANNEL_CAPACITY: usize = 8;
/// Buffers in the frame pool: what Graphics Capture may hold while we copy.
const POOL_BUFFERS: i32 = 3;
/// How long the capture thread sleeps with nothing to do before looking at the
/// stop flag again.
const IDLE_WAIT: Duration = Duration::from_millis(100);

fn backend(context: &str, e: windows::core::Error) -> CaptureError {
    CaptureError::Backend(format!("{context}: {e}"))
}

/// Start the Windows Runtime on this thread (free-threaded). A thread that
/// already did, in whatever mode, is fine.
fn init_runtime() {
    // SAFETY: plain call; the result only says whether it was already initialized.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

/// Graphics Capture's item for the primary monitor.
fn primary_monitor_item() -> Result<GraphicsCaptureItem, CaptureError> {
    if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
        return Err(CaptureError::Backend(
            "esta versión de Windows no permite capturar la pantalla (hace falta Windows 10 1803 o más nuevo)"
                .into(),
        ));
    }
    // SAFETY: plain Win32/WinRT calls; the point is a local.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        if monitor.is_invalid() {
            return Err(CaptureError::NoDisplay);
        }
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| backend("capture item factory", e))?;
        interop
            .CreateForMonitor(monitor)
            .map_err(|e| backend("capture the primary monitor", e))
    }
}

/// The primary monitor's size in physical pixels, whatever the DPI awareness
/// of this process.
pub fn primary_display_size() -> Result<(u32, u32), CaptureError> {
    init_runtime();
    let size = primary_monitor_item()?
        .Size()
        .map_err(|e| backend("monitor size", e))?;
    Ok((size.Width.max(0) as u32, size.Height.max(0) as u32))
}

#[derive(Debug, Clone)]
pub struct WindowsCaptureConfig {
    /// Size of the frames delivered (the screen is scaled to it).
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub show_cursor: bool,
}

impl Default for WindowsCaptureConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
            show_cursor: true,
        }
    }
}

/// [`FrameSource`] for the primary monitor, backed by Windows.Graphics.Capture.
pub struct WindowsGraphicsCaptureSource {
    config: WindowsCaptureConfig,
    epoch: SharedEpoch,
    running: Arc<AtomicBool>,
    /// Set by the frame-arrived callback; the capture thread waits on it.
    wake: Arc<(Mutex<bool>, Condvar)>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl WindowsGraphicsCaptureSource {
    /// `epoch` is shared with the audio source, which must stamp its chunks on
    /// the same clock.
    pub fn new(config: WindowsCaptureConfig, epoch: SharedEpoch) -> Self {
        Self {
            config,
            epoch,
            running: Arc::new(AtomicBool::new(false)),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            delivered: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            thread: None,
        }
    }
}

/// Everything living on the capture thread.
struct Session {
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    winrt_device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    capture: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    /// The size the pool was created for; `Recreate` when the screen changes.
    pool_size: windows::Graphics::SizeInt32,
    handler_token: i64,
    /// CPU-readable copy target, remade when the frame size changes.
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

fn create_device(
    driver: D3D_DRIVER_TYPE,
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    // SAFETY: the out pointers are locals that outlive the call.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(windows::core::Error::from(
            windows::Win32::Foundation::E_FAIL,
        )),
    }
}

impl Session {
    fn open(
        config: &WindowsCaptureConfig,
        wake: Arc<(Mutex<bool>, Condvar)>,
    ) -> Result<Self, CaptureError> {
        init_runtime();
        let item = primary_monitor_item()?;

        // A real GPU when there is one; the software rasterizer (WARP) on a
        // virtual machine or a server.
        let (device, context) = create_device(D3D_DRIVER_TYPE_HARDWARE)
            .or_else(|_| create_device(D3D_DRIVER_TYPE_WARP))
            .map_err(|e| backend("create the Direct3D device", e))?;
        let dxgi: IDXGIDevice = device.cast().map_err(|e| backend("DXGI device", e))?;
        // SAFETY: `dxgi` is a live device.
        let winrt_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .and_then(|inspectable| inspectable.cast())
            .map_err(|e| backend("Windows Runtime device", e))?;

        let pool_size = item.Size().map_err(|e| backend("monitor size", e))?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            POOL_BUFFERS,
            pool_size,
        )
        .map_err(|e| backend("frame pool", e))?;

        let handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                let (flag, signal) = &*wake;
                *flag.lock().expect("wake lock") = true;
                signal.notify_one();
                Ok(())
            });
        let handler_token = pool
            .FrameArrived(&handler)
            .map_err(|e| backend("frame arrived handler", e))?;

        let capture = pool
            .CreateCaptureSession(&item)
            .map_err(|e| backend("capture session", e))?;
        capture
            .SetIsCursorCaptureEnabled(config.show_cursor)
            .map_err(|e| backend("cursor capture", e))?;
        // The yellow border around what is being captured: Windows 11 lets an
        // app hide it. Best effort.
        let _ = capture.SetIsBorderRequired(false);
        capture
            .StartCapture()
            .map_err(|e| backend("start capture", e))?;

        Ok(Self {
            context,
            device,
            winrt_device,
            pool,
            capture,
            item,
            pool_size,
            handler_token,
            staging: None,
        })
    }

    /// A CPU-readable texture of this size, reusing the last one if it fits.
    fn staging_for(
        &mut self,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<ID3D11Texture2D, CaptureError> {
        if let Some((texture, w, h)) = &self.staging {
            if *w == desc.Width && *h == desc.Height {
                return Ok(texture.clone());
            }
        }
        let mut staging_desc = *desc;
        staging_desc.Usage = D3D11_USAGE_STAGING;
        staging_desc.BindFlags = 0;
        staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        staging_desc.MiscFlags = 0;
        staging_desc.MipLevels = 1;
        staging_desc.ArraySize = 1;
        let mut texture = None;
        // SAFETY: the descriptor is fully initialized; the out pointer is a local.
        unsafe {
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut texture))
        }
        .map_err(|e| backend("staging texture", e))?;
        let texture = texture.ok_or_else(|| CaptureError::Backend("no staging texture".into()))?;
        self.staging = Some((texture.clone(), desc.Width, desc.Height));
        Ok(texture)
    }

    /// Copy one captured frame into a BGRA picture of `out_w`×`out_h`.
    fn read_frame(
        &mut self,
        frame: &Direct3D11CaptureFrame,
        out_w: u32,
        out_h: u32,
    ) -> Result<Vec<u8>, CaptureError> {
        let surface = frame.Surface().map_err(|e| backend("frame surface", e))?;
        let access: IDirect3DDxgiInterfaceAccess =
            surface.cast().map_err(|e| backend("surface access", e))?;
        // SAFETY: the surface is a Direct3D 11 texture, as the pool was made for.
        let texture: ID3D11Texture2D =
            unsafe { access.GetInterface() }.map_err(|e| backend("frame texture", e))?;
        let content = frame
            .ContentSize()
            .map_err(|e| backend("content size", e))?;

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `desc` is a local the call fills in.
        unsafe { texture.GetDesc(&mut desc) };
        let staging = self.staging_for(&desc)?;

        // SAFETY: both textures are live and the same size and format; the
        // mapped pointer is valid until `Unmap`, and everything read from it is
        // copied out before that.
        unsafe {
            self.context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| backend("map the frame", e))?;
            let width = (content.Width.max(1) as u32).min(desc.Width) as usize;
            let height = (content.Height.max(1) as u32).min(desc.Height) as usize;
            let stride = mapped.RowPitch as usize;
            let src = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                stride * (height - 1) + width * 4,
            );
            let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
            scale_bgra(
                src,
                width,
                height,
                stride,
                &mut out,
                out_w as usize,
                out_h as usize,
            );
            self.context.Unmap(&staging, 0);
            Ok(out)
        }
    }

    /// The screen changed size: resize the pool to match.
    fn follow_size(&mut self, frame: &Direct3D11CaptureFrame) {
        let Ok(content) = frame.ContentSize() else {
            return;
        };
        let changed =
            content.Width != self.pool_size.Width || content.Height != self.pool_size.Height;
        if changed
            && self
                .pool
                .Recreate(
                    &self.winrt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    POOL_BUFFERS,
                    content,
                )
                .is_ok()
        {
            self.pool_size = content;
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.handler_token);
        let _ = self.capture.Close();
        let _ = self.pool.Close();
        let _ = &self.item;
    }
}

impl FrameSource for WindowsGraphicsCaptureSource {
    fn start(&mut self) -> Result<mpsc::Receiver<RawFrame>, CaptureError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CaptureError::AlreadyStarted);
        }
        let (width, height, fps) = (self.config.width, self.config.height, self.config.fps);
        if fps == 0 || width < 2 || height < 2 {
            self.running.store(false, Ordering::SeqCst);
            return Err(CaptureError::Backend("bad capture size or rate".into()));
        }
        self.delivered.store(0, Ordering::SeqCst);
        self.dropped.store(0, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), CaptureError>>();

        let config = self.config.clone();
        let epoch = self.epoch.clone();
        let running = Arc::clone(&self.running);
        let wake = Arc::clone(&self.wake);
        let delivered = Arc::clone(&self.delivered);
        let dropped = Arc::clone(&self.dropped);

        self.thread = Some(std::thread::spawn(move || {
            let mut session = match Session::open(&config, Arc::clone(&wake)) {
                Ok(session) => {
                    let _ = ready_tx.send(Ok(()));
                    session
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    running.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let mut pacer = Pacer::new(config.fps);
            let mut last_ns: i64 = -1;
            let mut frame_number: u64 = 0;
            while running.load(Ordering::SeqCst) {
                {
                    let (flag, signal) = &*wake;
                    let mut ready = flag.lock().expect("wake lock");
                    if !*ready {
                        ready = signal.wait_timeout(ready, IDLE_WAIT).expect("wake wait").0;
                    }
                    *ready = false;
                }
                // Take everything that is waiting (the pool holds a few).
                while let Ok(frame) = session.pool.TryGetNextFrame() {
                    session.follow_size(&frame);
                    let Ok(time) = frame.SystemRelativeTime() else {
                        continue;
                    };
                    let ns = time.Duration.saturating_mul(100);
                    let rel_ns = ns - epoch.get_or_set(ns);
                    // strictly increasing, and no faster than the target rate
                    if rel_ns <= last_ns || !pacer.accept(rel_ns) {
                        continue;
                    }
                    match session.read_frame(&frame, config.width, config.height) {
                        Ok(data) => {
                            let out = RawFrame {
                                data,
                                width: config.width,
                                height: config.height,
                                timestamp: Duration::from_nanos(rel_ns.max(0) as u64),
                                frame_number,
                            };
                            match tx.try_send(out) {
                                Ok(()) => {
                                    last_ns = rel_ns;
                                    frame_number += 1;
                                    delivered.fetch_add(1, Ordering::SeqCst);
                                }
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    dropped.fetch_add(1, Ordering::SeqCst);
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    running.store(false, Ordering::SeqCst);
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("clipforge: could not read a captured frame: {e}");
                            dropped.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            }
            drop(session);
            // `tx` drops here, which ends the stream for the consumer.
        }));

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(rx),
            Ok(Err(e)) => {
                self.stop();
                Err(e)
            }
            Err(_) => {
                self.stop();
                Err(CaptureError::Backend("capture thread exited early".into()))
            }
        }
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.wake.1.notify_all();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    fn stats(&self) -> CaptureStats {
        CaptureStats {
            delivered: self.delivered.load(Ordering::SeqCst),
            dropped: self.dropped.load(Ordering::SeqCst),
        }
    }

    fn epoch(&self) -> Option<SharedEpoch> {
        Some(self.epoch.clone())
    }
}

impl Drop for WindowsGraphicsCaptureSource {
    fn drop(&mut self) {
        self.stop();
    }
}
