//! Per-call state that lives on the calling thread instead of in the [`Session`], so that
//! several read-only commands can run on one shared `&Session` at once
//! ([`Session::execute_shared`], `docs/web-deployment.md` "Concurrency"):
//!
//! - the call's **cancel flag**, seen by [`Session::is_cancelled`] next to the session's own;
//! - the **view overrides** of `report.render {ui_language, culture, as_of}`: the render draws
//!   under them without writing them into the session (it used to swap the session's fields and
//!   put them back, which needs `&mut Session`).
//!
//! Both are read only on the thread that runs the command (the DAX evaluator checks the cancel
//! flag where it checks its time budget, on that thread; the clock and the UI language are read
//! before any worker starts). Each setter returns a guard that restores the previous value, so
//! scopes nest and a panic unwinding through a command leaves nothing behind.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Session, UiLanguage};

#[derive(Default, Clone)]
struct Scope {
    cancel: Option<Arc<AtomicBool>>,
    ui_language: Option<UiLanguage>,
    /// `Some(x)`: the display culture is `x` for this call (`Some(None)`: the stored names).
    display_culture: Option<Option<String>>,
    as_of: Option<f64>,
    /// `report.render {key_influencers}`: `Some(true)` draws the BIcraft estimate.
    key_influencers: Option<bool>,
    /// `Some(x)`: the View as state of this call is `x` (a viewer's own state over a shared
    /// session, `Session::snapshot_view`); `None`: the session's.
    view_as: Option<Option<crate::interact::ViewAs>>,
    /// The session's `dax.cancel` epoch when this call's `dax.evaluate` started
    /// (`dax_queries.rs`).
    dax_epoch: Option<u64>,
}

thread_local! {
    static SCOPE: RefCell<Scope> = RefCell::new(Scope::default());
}

/// Restores the scope it replaced when dropped.
pub(crate) struct Guard(Option<Scope>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            SCOPE.with(|s| *s.borrow_mut() = prev);
        }
    }
}

fn change(f: impl FnOnce(&mut Scope)) -> Guard {
    SCOPE.with(|s| {
        let mut s = s.borrow_mut();
        let prev = s.clone();
        f(&mut s);
        Guard(Some(prev))
    })
}

/// The call's cancel flag for the guard's lifetime.
pub(crate) fn cancel(flag: Option<Arc<AtomicBool>>) -> Guard {
    change(|s| s.cancel = flag)
}

/// The view overrides of one render for the guard's lifetime (each `None`: the session's).
pub(crate) fn view(
    ui_language: Option<UiLanguage>,
    display_culture: Option<Option<String>>,
    as_of: Option<f64>,
) -> Guard {
    change(|s| {
        if ui_language.is_some() {
            s.ui_language = ui_language;
        }
        if display_culture.is_some() {
            s.display_culture = display_culture;
        }
        if as_of.is_some() {
            s.as_of = as_of;
        }
    })
}

/// The `dax.cancel` epoch a `dax.evaluate` started at, for the guard's lifetime.
pub(crate) fn dax_epoch(epoch: u64) -> Guard {
    change(|s| s.dax_epoch = Some(epoch))
}

/// The epoch the running `dax.evaluate` started at (`None`: no evaluation on this thread).
pub(crate) fn dax_epoch_now() -> Option<u64> {
    SCOPE.with(|s| s.borrow().dax_epoch)
}

/// The call's cancel flag is raised.
pub(crate) fn cancelled() -> bool {
    SCOPE.with(|s| {
        s.borrow()
            .cancel
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
    })
}

impl Session {
    /// The UI language the current call draws in: a render's override, else the session's.
    pub(crate) fn ui_language_now(&self) -> UiLanguage {
        SCOPE
            .with(|s| s.borrow().ui_language.clone())
            .unwrap_or_else(|| self.ui_language.clone())
    }

    /// The display culture of the current call: a render's override, else the session's.
    pub(crate) fn display_culture_now(&self) -> Option<String> {
        SCOPE
            .with(|s| s.borrow().display_culture.clone())
            .unwrap_or_else(|| self.display_culture.clone())
    }

    /// The `as_of` of the current call: a render's override, else the session's.
    pub(crate) fn as_of_now(&self) -> Option<f64> {
        SCOPE.with(|s| s.borrow().as_of).or(self.as_of)
    }

    /// The View as state of the current call: a viewer's (snapshot) override, else the
    /// session's (`view.interact {action: "view_as"}`).
    pub(crate) fn view_as_now(&self) -> Option<crate::interact::ViewAs> {
        SCOPE
            .with(|s| s.borrow().view_as.clone())
            .unwrap_or_else(|| self.interactions.view_as().cloned())
    }
}

/// The View as state of one view for the guard's lifetime (`None`: no View as, whatever the
/// session's).
pub(crate) fn view_as(state: Option<crate::interact::ViewAs>) -> Guard {
    change(|s| s.view_as = Some(state))
}

/// `report.render {key_influencers}` for the guard's lifetime (`None`: the viewer's state).
pub(crate) fn key_influencers(estimate: Option<bool>) -> Guard {
    change(|s| {
        if estimate.is_some() {
            s.key_influencers = estimate;
        }
    })
}

/// The current call's key influencers override, if any.
pub(crate) fn key_influencers_now() -> Option<bool> {
    SCOPE.with(|s| s.borrow().key_influencers)
}
