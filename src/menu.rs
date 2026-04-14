use core::fmt::{Debug, Write};

use embassy_time::{Duration, Instant};
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use epd_waveshare::{color::Color, epd1in54_v2::Display1in54};
use esp_hal::rtc_cntl::Rtc;

use crate::buttons::{ButtonChange, ButtonEvent, ButtonId, Buttons};

const FONT: MonoFont = FONT_10X20;
const STYLE: MonoTextStyle<'_, Color> = MonoTextStyle::new(&FONT, Color::Black);

pub trait MenuItem: core::fmt::Debug + Eq + Clone {
    fn render(&self, display: &mut Display1in54, time: &RtcTimeProvider);
    fn update(self, time: &RtcTimeProvider, change: ButtonChange, buttons: &Buttons) -> OneOfMenu;
}

#[derive(Debug, PartialEq, Eq, Clone)]
/// Utility to workaround not having dynamic dispatch on a `&dyn MenuItem`,
/// because I'm avoiding using RAM.
pub enum OneOfMenu {
    MenuMain(MenuMain),
    TimeSet(TimeSet),
    DebugView(DebugView),
}
impl MenuItem for OneOfMenu {
    fn render(&self, display: &mut Display1in54, time: &RtcTimeProvider) {
        match self {
            Self::MenuMain(m) => m.render(display, time),
            Self::TimeSet(m) => m.render(display, time),
            Self::DebugView(m) => m.render(display, time),
        }
    }
    fn update(self, time: &RtcTimeProvider, change: ButtonChange, buttons: &Buttons) -> OneOfMenu {
        match self {
            Self::MenuMain(m) => m.update(time, change, buttons),
            Self::TimeSet(m) => m.update(time, change, buttons),
            Self::DebugView(m) => m.update(time, change, buttons),
        }
    }
}

pub struct RtcTimeProvider<'a> {
    rtc: &'a Rtc<'a>,
}
impl<'a> RtcTimeProvider<'a> {
    pub fn new(rtc: &'a Rtc<'a>) -> Self {
        RtcTimeProvider { rtc }
    }
    pub fn get_current_time(&self) -> Instant {
        Instant::from_micros(self.rtc.current_time_us())
    }
    pub fn set_current_time(&self, time: Instant) {
        self.rtc.set_current_time_us(time.as_micros());
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MenuMain;
impl MenuItem for MenuMain {
    fn render(&self, display: &mut Display1in54, time: &RtcTimeProvider) {
        let current_time = time.get_current_time();
        let mut s = heapless::String::<20>::new();
        let _ = write!(
            s,
            "{:02}:{:02}",
            current_time.as_secs() / 3600 % 24,
            current_time.as_secs() / 60 % 60,
        );
        let text = Text::new(&s, Point::new(70, 40), STYLE);
        text.draw(display).unwrap();
    }
    fn update(self, _t: &RtcTimeProvider, change: ButtonChange, _b: &Buttons) -> OneOfMenu {
        if let (ButtonId::Button1, ButtonEvent::Pressed) = change {
            OneOfMenu::TimeSet(TimeSet)
        } else {
            OneOfMenu::MenuMain(self)
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TimeSet;
impl MenuItem for TimeSet {
    fn render(&self, display: &mut Display1in54, time: &RtcTimeProvider) {
        let text = Text::new("time set mode", Point::new(35, 100), STYLE);
        text.draw(display).unwrap();
        // Hacky, but render the current time in the time set screen too.
        MenuMain.render(display, time);
    }
    fn update(self, time: &RtcTimeProvider, change: ButtonChange, buttons: &Buttons) -> OneOfMenu {
        match change {
            // Save and return
            (ButtonId::Button1, ButtonEvent::Pressed) => return OneOfMenu::MenuMain(MenuMain),
            // Adjust time
            (b, ButtonEvent::Pressed) if b == ButtonId::Button2 || b == ButtonId::Button3 => {
                let mut t = time.get_current_time();
                let shift = Duration::from_secs(if b == ButtonId::Button2 { 60 * 60 } else { 60 });
                t = if buttons.is_pressed(ButtonId::Button4) {
                    t.saturating_sub(shift)
                } else {
                    t.saturating_add(shift)
                };
                time.set_current_time(t);
            }
            _ => {}
        }
        OneOfMenu::TimeSet(self)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DebugView {
    message: heapless::String<100>,
}
impl MenuItem for DebugView {
    fn render(&self, display: &mut Display1in54, _: &RtcTimeProvider) {
        let text = Text::new(&self.message, Point::new(10, 10), STYLE);
        text.draw(display).unwrap();
    }
    fn update(self, _t: &RtcTimeProvider, _c: ButtonChange, _b: &Buttons) -> OneOfMenu {
        OneOfMenu::DebugView(self)
    }
}
