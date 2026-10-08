use std::sync::Arc;

use xilem::{Color, WidgetView};
use xilem::style::Style;
use xilem::view::{
    FlexSpacer,
    flex_row,
    prose,
    text_button,
};

#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => {{
        let msg = format!($($arg)+);
        log::error!("{}", msg);
        Banner::error(msg)
    }}
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => {{
        let msg = format!($($arg)+);
        log::warn!("{}", msg);
        Banner::warning(msg)
    }}
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => {{
        let msg = format!($($arg)+);
        log::info!("{}", msg);
        Banner::info(msg)
    }}
}

#[derive(Debug)]
pub struct Banner {
    pub text: Arc<str>,
    pub severity: Severity,
}

impl Banner {
    pub fn error<S: Into<Arc<str>>>(text: S) -> Self {
        Self {
            text: text.into(),
            severity: Severity::Error,
        }
    }

    pub fn warning<S: Into<Arc<str>>>(text: S) -> Self {
        Self {
            text: text.into(),
            severity: Severity::Warning,
        }
    }

    #[allow(unused)]
    pub fn info<S: Into<Arc<str>>>(text: S) -> Self {
        Self {
            text: text.into(),
            severity: Severity::Info,
        }
    }

    pub fn opt_display<ParentState, F>(banner: &mut Option<Self>, f: F)
        -> Option<impl WidgetView<ParentState> + use <ParentState, F>>
    where
        ParentState: 'static,
        F: for<'a> Fn(&'a mut ParentState) -> &'a mut Option<Self> + Send + Sync + 'static,
    {
        match banner {
            None => None,
            Some(m) => {
                let view = flex_row((
                    prose(Arc::clone(&m.text))
                        .text_color(m.severity.fg_color()),
                    FlexSpacer::Flex(2.0),
                    text_button("Dismiss", |banner: &mut Option<Banner>| *banner = None)
                )).background(m.severity.bg_color());
                Some(xilem::core::map_state(view, f))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    #[allow(unused)]
    Error,
    #[allow(unused)]
    Warning,
    #[allow(unused)]
    Info,
}

impl Severity {
    fn bg_color(&self) -> Color {
        match self {
            // tailwind red 400
            Self::Error => Color::new([0.972549, 0.44313726, 0.44313726, 1.0]),
            // tailwind amber 400
            Self::Warning => Color::new([0.9843137, 0.7490196, 0.14117648, 1.0]),
            // tailwind blue 300
            Self::Info => Color::new([0.5764706, 0.77254903, 0.99215686, 1.0]),
        }
    }

    fn fg_color(&self) -> Color {
        Color::BLACK
    }
}
