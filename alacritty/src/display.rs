
//! The display subsystem including window management, font rasterization, and
//! GPU drawing.
use std::sync::mpsc;

use parking_lot::MutexGuard;

use Rgb;
use config::{self, Config};
use font::{self, Rasterize};
use meter::Meter;
use renderer::{self, GlyphCache, QuadRenderer};
use term::{Term, SizeInfo, RenderableCell};
use sync::FairMutex;

use window::{self, Pixels, Size};

#[derive(Debug)]
pub enum Error {
    /// Error with window management
    Window(window::Error),

    /// Error dealing with fonts
    Font(font::Error),

    /// Error in renderer
    Render(renderer::Error),
}

impl ::std::error::Error for Error {
    fn cause(&self) -> Option<&::std::error::Error> {
        match *self {
            Error::Window(ref err) => Some(err),
            Error::Font(ref err) => Some(err),
            Error::Render(ref err) => Some(err),
        }
    }

    fn description(&self) -> &str {
        match *self {
            Error::Window(ref err) => err.description(),
            Error::Font(ref err) => err.description(),
            Error::Render(ref err) => err.description(),
        }
    }
}

impl ::std::fmt::Display for Error {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        match *self {
            Error::Window(ref err) => err.fmt(f),
            Error::Font(ref err) => err.fmt(f),
            Error::Render(ref err) => err.fmt(f),
        }
    }
}

impl From<window::Error> for Error {
    fn from(val: window::Error) -> Error {
        Error::Window(val)
    }
}

impl From<font::Error> for Error {
    fn from(val: font::Error) -> Error {
        Error::Font(val)
    }
}

impl From<renderer::Error> for Error {
    fn from(val: renderer::Error) -> Error {
        Error::Render(val)
    }
}

pub enum DisplayCommand {
    NewSize(u32, u32),
    NewHiDPIFactor(f32),
}

/// The display wraps a font rasterizer and GPU renderer
pub struct Display {
    renderer: QuadRenderer,
    glyph_cache: GlyphCache,
    render_timer: bool,
    rx: mpsc::Receiver<DisplayCommand>,
    tx: mpsc::Sender<DisplayCommand>,
    meter: Meter,
    font_size: font::Size,
    size_info: SizeInfo,
}

/// Types that are interested in when the display is resized
pub trait OnResize {
    fn on_resize(&mut self, size: &SizeInfo);
}

pub enum InitialSize {
    Cells(config::Dimensions),
    Pixels(Size<Pixels<u32>>),
}

impl Display {
    pub fn update_config(&mut self, config: &Config) {
        self.render_timer = config.render_timer();
    }

    /// Get size info about the display
    pub fn size(&self) -> &SizeInfo {
        &self.size_info
    }

    pub fn new(
        config: &Config,
        size: InitialSize,
        dpr: f32
    ) -> Result<Display, Error> {
        // Extract some properties from config
        let render_timer = config.render_timer();

        // Create renderer
        // Start with zero size, then initialize the font rasterizer, compute font metrics and use
        // those to calculate the actual size if needed.
        let zero_size = Size { width: Pixels(0), height: Pixels(0) };
        let mut renderer = QuadRenderer::new(&config, zero_size)?;
        let (glyph_cache, cell_width, cell_height) =
            Self::new_glyph_cache(dpr, &mut renderer, config)?;
        let size = match size {
            InitialSize::Cells(dimensions) => {
                let width = cell_width as u32 * dimensions.columns_u32();
                let height = cell_height as u32 * dimensions.lines_u32();
                Size {
                    width: Pixels(width + 2 * u32::from(config.padding().x)),
                    height: Pixels(height + 2 * u32::from(config.padding().y)),
                }
            },
            InitialSize::Pixels(size) => size,
        };
        renderer.resize(size.width.0 as _, size.height.0 as _);
        info!("Cell Size: ({} x {})", cell_width, cell_height);

        let size_info = SizeInfo {
            width: size.width.0 as f32,
            height: size.height.0 as f32,
            cell_width: cell_width as f32,
            cell_height: cell_height as f32,
            padding_x: f32::from(config.padding().x),
            padding_y: f32::from(config.padding().y),
        };

        // Channel for resize events
        //
        // macOS has a callback for getting resize events, the channel is used
        // to queue resize events until the next draw call. Unfortunately, it
        // seems that the event loop is blocked until the window is done
        // resizing. If any drawing were to happen during a resize, it would
        // need to be in the callback.
        let (tx, rx) = mpsc::channel();

        // Clear screen
        let background_color = config.colors().primary.background;
        renderer.with_api(
            config,
            &size_info,
            0., /* visual bell intensity */
            |api| {
                api.clear(background_color);
            },
        );

        Ok(Display {
            renderer,
            glyph_cache,
            render_timer,
            tx,
            rx,
            meter: Meter::new(),
            font_size: font::Size::new(0.),
            size_info,
        })
    }

    fn new_glyph_cache(dpr: f32, renderer: &mut QuadRenderer, config: &Config)
        -> Result<(GlyphCache, f32, f32), Error>
    {
        let font = config.font().clone();
        let rasterizer = font::Rasterizer::new(dpr, config.use_thin_strokes())?;

        // Initialize glyph cache
        let glyph_cache = {
            info!("Initializing glyph cache");
            let init_start = ::std::time::Instant::now();

            let cache =
                renderer.with_loader(|mut api| GlyphCache::new(rasterizer, &font, &mut api))?;

            let stop = init_start.elapsed();
            let stop_f = stop.as_secs() as f64 + f64::from(stop.subsec_nanos()) / 1_000_000_000f64;
            info!("Finished initializing glyph cache in {}", stop_f);

            cache
        };

        // Need font metrics to resize the window properly. This suggests to me the
        // font metrics should be computed before creating the window in the first
        // place so that a resize is not needed.
        let metrics = glyph_cache.font_metrics();
        let cell_width = metrics.average_advance as f32 + f32::from(font.offset().x);
        let cell_height = metrics.line_height as f32 + f32::from(font.offset().y);

        // Prevent invalid cell sizes
        if cell_width < 1. || cell_height < 1. {
            panic!("font offset is too small");
        }

        Ok((glyph_cache, cell_width.floor(), cell_height.floor()))
    }

    pub fn update_glyph_cache(&mut self, config: &Config, new_dpr: Option<f32>) {
        let cache = &mut self.glyph_cache;
        let size = self.font_size;
        self.renderer.with_loader(|mut api| {
            let _ = cache.update_font_size(config.font(), size, new_dpr, &mut api);
        });

        let metrics = cache.font_metrics();
        self.size_info.cell_width = ((metrics.average_advance + f64::from(config.font().offset().x)) as f32).floor();
        self.size_info.cell_height = ((metrics.line_height + f64::from(config.font().offset().y)) as f32).floor();
    }

    #[inline]
    pub fn command_channel(&self) -> mpsc::Sender<DisplayCommand> {
        self.tx.clone()
    }

    /// Process pending resize (and HiDPI factor) events
    pub fn handle_resize(
        &mut self,
        terminal: &mut MutexGuard<Term>,
        config: &Config,
        items: &mut [&mut OnResize],
    ) {
        // Resize events new_size and are handled outside the poll_events
        // iterator. This has the effect of coalescing multiple resize
        // events into one.
        let mut new_size = None;
        let mut new_dpr = None;

        // Take most recent resize event, if any
        while let Ok(sz) = self.rx.try_recv() {
            match sz {
                DisplayCommand::NewSize(w, h) => new_size = Some((w, h)),
                DisplayCommand::NewHiDPIFactor(dpr) => new_dpr = Some(dpr)
            }
        }

        // Font size modification detected
        if terminal.font_size != self.font_size || new_dpr.is_some() {
            self.font_size = terminal.font_size;
            self.update_glyph_cache(config, new_dpr);

            if new_size == None {
                // Force a resize to refresh things
                new_size = Some((self.size_info.width as u32, self.size_info.height as u32));
            }
        }

        // Receive any resize events; only call gl::Viewport on last
        // available
        if let Some((w, h)) = new_size.take() {
            self.size_info.width = w as f32;
            self.size_info.height = h as f32;

            let size = &self.size_info;
            terminal.resize(size);

            for item in items {
                item.on_resize(size)
            }

            self.renderer.resize(w as i32, h as i32);
        }
    }

    /// Draw the screen
    ///
    /// A reference to Term whose state is being drawn must be provided.
    ///
    /// This call may block if vsync is enabled
    pub fn draw(&mut self, terminal: &FairMutex<Term>, config: &Config, window_focused: bool) {
        let mut terminal = terminal.lock();
        let size_info = *terminal.size_info();
        let visual_bell_intensity = terminal.visual_bell.intensity();

        let grid_cells: Vec<RenderableCell> = terminal
            .renderable_cells(config, window_focused)
            .collect();

        // Clear dirty flag
        terminal.dirty = !terminal.visual_bell.completed();

        {
            let glyph_cache = &mut self.glyph_cache;

            // Draw grid
            {
                let _sampler = self.meter.sampler();

                self.renderer.with_api(config, &size_info, visual_bell_intensity, |mut api| {
                    api.clear(terminal.background_color());

                    // Draw the grid
                    api.render_cells(grid_cells.iter(), glyph_cache);
                });
            }

            // Draw render timer
            if self.render_timer {
                let timing = format!("{:.3} usec", self.meter.average());
                let color = Rgb {
                    r: 0xd5,
                    g: 0x4e,
                    b: 0x53,
                };
                self.renderer
                    .with_api(config, &size_info, visual_bell_intensity, |mut api| {
                        api.render_string(&timing[..], glyph_cache, color);
                    });
            }
        }
    }

    /// Adjust the XIM editor position according to the new location of the cursor
    pub fn current_xim_spot(&mut self, terminal: &Term) -> (i32, i32) {
        use index::{Column, Line, Point};
        use term::SizeInfo;
        let Point{line: Line(row), col: Column(col)} = terminal.cursor().point;
        let SizeInfo{cell_width: cw,
                    cell_height: ch,
                    padding_x: px,
                    padding_y: py, ..} = *terminal.size_info();
        let nspot_y = (py + (row + 1) as f32 * ch) as i32;
        let nspot_x = (px + col as f32 * cw) as i32;
        (nspot_x, nspot_y)
    }
}

