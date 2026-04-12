use core::cell::RefCell;
use core::panic;

use critical_section::Mutex;
use defmt::{error, info};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, TrySendError};
use esp_hal::gpio::{Event, Input, InputConfig, InputPin, Io, Pull};
use esp_hal::handler;
use esp_hal::peripherals::{GPIO0, GPIO6, GPIO7, GPIO8};

pub type ButtonPins = (
    GPIO7<'static>,
    GPIO6<'static>,
    GPIO0<'static>,
    GPIO8<'static>,
);

/// Button events. Lives for whole program duration: cleared upon creating a `Buttons` object and
/// can't be accessed after that object is reclaimed.
static CHANNEL: Channel<CriticalSectionRawMutex, (ButtonId, ButtonEvent), 100> = Channel::new();
#[derive(Debug, defmt::Format, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    Button1,
    Button2,
    Button3,
    Button4,
}
#[derive(Debug, defmt::Format, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEvent {
    Pressed,
    Released,
}

/// Button states, stored globally so that they can be accessed from the interrupt handler.
static BUTTONS: Mutex<RefCell<Option<SharedButtons>>> = Mutex::new(RefCell::new(None));
struct SharedButtons {
    pub button1: ButtonState<GPIO7<'static>>,
    pub button2: ButtonState<GPIO6<'static>>,
    pub button3: ButtonState<GPIO0<'static>>,
    pub button4: ButtonState<GPIO8<'static>>,
}

/// Helper wrapper for dealing with configuring and reclaiming GPIO pins as buttons.
pub struct Buttons;
impl Buttons {
    /// Can only be called if not called before, or if `reclaim` has been called on the
    /// object returned by the first call.
    pub fn init(io: &mut Io, (btn1, btn2, btn3, btn4): ButtonPins) -> Self {
        critical_section::with(|cs| {
            let mut b = BUTTONS.borrow_ref_mut(cs);
            if b.is_some() {
                panic!("`init` called again before `reclaim` was called");
            }

            io.set_interrupt_handler(button_gpio_handler);

            CHANNEL.clear();
            let buttons = unsafe {
                SharedButtons {
                    button1: ButtonState::new(ButtonId::Button1, btn1.clone_unchecked(), btn1),
                    button2: ButtonState::new(ButtonId::Button2, btn2.clone_unchecked(), btn2),
                    button3: ButtonState::new(ButtonId::Button3, btn3.clone_unchecked(), btn3),
                    button4: ButtonState::new(ButtonId::Button4, btn4.clone_unchecked(), btn4),
                }
            };

            b.replace(buttons);
        });
        Self
    }

    pub fn reclaim(self) -> ButtonPins {
        let Some(b) = critical_section::with(|cs| BUTTONS.take(cs)) else {
            panic!("`reclaim` called before `init` was called");
        };
        // We don't need to clear the IO interrupt handler, but we do need to
        // stop each button from listening - this is handled by reclaim.
        (
            b.button1.reclaim(),
            b.button2.reclaim(),
            b.button3.reclaim(),
            b.button4.reclaim(),
        )
    }

    pub async fn wait_for_event(&mut self) -> (ButtonId, ButtonEvent) {
        CHANNEL.receive().await
    }
    pub fn get_states(&self) -> (bool, bool, bool, bool) {
        critical_section::with(|cs| {
            let b = BUTTONS.borrow_ref(cs);
            let Some(buttons) = b.as_ref() else {
                panic!("`reclaim` called before `init` was called");
            };
            (
                buttons.button1.input.is_low(),
                buttons.button2.input.is_low(),
                buttons.button3.input.is_low(),
                buttons.button4.input.is_low(),
            )
        })
    }
}

pub struct ButtonState<P> {
    id: ButtonId,
    /// This pin is an unsafely duplicated pin, which must not be used
    /// at the same time as the pin held by `input`. We need to store this
    /// since we can't get the pin back out of the `input`.
    /// This module enforces the "only one pin used" rule.
    pin: P,
    input: Input<'static>,
    pressed: bool,
}
impl<P> ButtonState<P>
where
    P: InputPin + 'static,
{
    fn new(id: ButtonId, pin: P, unsafe_pin: P) -> Self {
        let mut input = Input::new(pin, InputConfig::default().with_pull(Pull::Up));
        input.listen(Event::AnyEdge);
        Self {
            id,
            pin: unsafe_pin,
            input,
            pressed: false,
        }
    }
    fn handle_interrupt(&mut self) {
        if !self.input.is_interrupt_set() {
            return;
        }
        self.input.clear_interrupt();

        if !self.pressed && self.input.is_low() {
            self.pressed = true;
            if let Err(TrySendError::Full(_)) = CHANNEL.try_send((self.id, ButtonEvent::Pressed)) {
                error!("Button channel was full");
            }
        } else if self.pressed && self.input.is_high() {
            self.pressed = false;
            if let Err(TrySendError::Full(_)) = CHANNEL.try_send((self.id, ButtonEvent::Released)) {
                error!("Button channel was full");
            }
        }
    }
    pub fn reclaim(mut self) -> P {
        self.input.unlisten();
        self.pin
    }
}

#[handler]
fn button_gpio_handler() {
    critical_section::with(|cs| {
        let mut s = BUTTONS.borrow_ref_mut(cs);
        let Some(state) = s.as_mut() else {
            // Don't panic: maybe in some interrupt race conditions the buttons
            // could have been reclaimed before we get here.
            error!("button gpio handler called before buttons initialised");
            return;
        };
        state.button1.handle_interrupt();
        state.button2.handle_interrupt();
        state.button3.handle_interrupt();
        state.button4.handle_interrupt();
    })
}
