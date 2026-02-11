//! Lifecycle hooks for the facilitator payment processing pipeline.
//!
//! Hooks allow applications to intercept and customize the facilitator's
//! verify and settle operations. This mirrors the Go SDK's
//! `facilitator_hooks.go` design with three hook points per operation:
//!
//! - **Before hooks** — Run before the operation; can abort it.
//! - **After hooks** — Run after success; errors are logged but don't affect the result.
//! - **Failure hooks** — Run on error; can recover with a substitute response.
//!
//! Use [`HookedSchemeHandler`] to wrap any [`SchemeHandler`] with hooks.

use crate::proto;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::handler::{SchemeHandler, SchemeHandlerError};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Context passed to facilitator verify hooks.
#[derive(Debug, Clone)]
pub struct FacilitatorVerifyContext {
    /// The raw verify request (contains both payload and requirements as JSON).
    pub request: proto::VerifyRequest,
}

/// Context passed to facilitator verify result hooks.
#[derive(Debug, Clone)]
pub struct FacilitatorVerifyResultContext {
    /// Original verify context.
    pub ctx: FacilitatorVerifyContext,
    /// The verification result.
    pub response: proto::VerifyResponse,
}

/// Context passed to facilitator verify failure hooks.
#[derive(Debug, Clone)]
pub struct FacilitatorVerifyFailureContext {
    /// Original verify context.
    pub ctx: FacilitatorVerifyContext,
    /// The error description.
    pub error: String,
}

/// Context passed to facilitator settle hooks.
#[derive(Debug, Clone)]
pub struct FacilitatorSettleContext {
    /// The raw settle request.
    pub request: proto::SettleRequest,
}

/// Context passed to facilitator settle result hooks.
#[derive(Debug, Clone)]
pub struct FacilitatorSettleResultContext {
    /// Original settle context.
    pub ctx: FacilitatorSettleContext,
    /// The settlement result.
    pub response: proto::SettleResponse,
}

/// Context passed to facilitator settle failure hooks.
#[derive(Debug, Clone)]
pub struct FacilitatorSettleFailureContext {
    /// Original settle context.
    pub ctx: FacilitatorSettleContext,
    /// The error description.
    pub error: String,
}

/// Result returned by a facilitator before-hook.
///
/// If `abort` is `true`, the operation is skipped and an error with `reason` is returned.
#[derive(Debug, Clone, Default)]
pub struct FacilitatorBeforeHookResult {
    /// Whether to abort the operation.
    pub abort: bool,
    /// Machine-readable reason for aborting.
    pub reason: String,
    /// Human-readable message.
    pub message: String,
}

/// Result returned by a facilitator verify failure hook.
///
/// If `recovered` is `true`, the `response` replaces the original error.
#[derive(Debug, Clone)]
pub struct FacilitatorVerifyFailureHookResult {
    /// Whether this hook recovered from the failure.
    pub recovered: bool,
    /// Replacement verify response (only used if `recovered` is `true`).
    pub response: proto::VerifyResponse,
}

/// Result returned by a facilitator settle failure hook.
///
/// If `recovered` is `true`, the `response` replaces the original error.
#[derive(Debug, Clone)]
pub struct FacilitatorSettleFailureHookResult {
    /// Whether this hook recovered from the failure.
    pub recovered: bool,
    /// Replacement settle response (only used if `recovered` is `true`).
    pub response: proto::SettleResponse,
}

/// Hook called before facilitator payment verification.
pub type BeforeVerifyHookFn = dyn Fn(
        FacilitatorVerifyContext,
    ) -> BoxFut<'static, Result<Option<FacilitatorBeforeHookResult>, String>>
    + Send
    + Sync;

/// Hook called after successful facilitator payment verification.
pub type AfterVerifyHookFn =
    dyn Fn(FacilitatorVerifyResultContext) -> BoxFut<'static, Result<(), String>> + Send + Sync;

/// Hook called when facilitator payment verification fails.
pub type OnVerifyFailureHookFn = dyn Fn(
        FacilitatorVerifyFailureContext,
    ) -> BoxFut<'static, Result<Option<FacilitatorVerifyFailureHookResult>, String>>
    + Send
    + Sync;

/// Hook called before facilitator payment settlement.
pub type BeforeSettleHookFn = dyn Fn(
        FacilitatorSettleContext,
    ) -> BoxFut<'static, Result<Option<FacilitatorBeforeHookResult>, String>>
    + Send
    + Sync;

/// Hook called after successful facilitator payment settlement.
pub type AfterSettleHookFn =
    dyn Fn(FacilitatorSettleResultContext) -> BoxFut<'static, Result<(), String>> + Send + Sync;

/// Hook called when facilitator payment settlement fails.
pub type OnSettleFailureHookFn = dyn Fn(
        FacilitatorSettleFailureContext,
    ) -> BoxFut<'static, Result<Option<FacilitatorSettleFailureHookResult>, String>>
    + Send
    + Sync;

/// Collection of lifecycle hooks for the facilitator processing pipeline.
///
/// All hooks are optional. Multiple hooks of the same type execute in
/// registration order.
#[derive(Clone, Default)]
pub struct SchemeHandlerHooks {
    before_verify: Vec<Arc<BeforeVerifyHookFn>>,
    after_verify: Vec<Arc<AfterVerifyHookFn>>,
    on_verify_failure: Vec<Arc<OnVerifyFailureHookFn>>,
    before_settle: Vec<Arc<BeforeSettleHookFn>>,
    after_settle: Vec<Arc<AfterSettleHookFn>>,
    on_settle_failure: Vec<Arc<OnSettleFailureHookFn>>,
}

impl std::fmt::Debug for SchemeHandlerHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemeHandlerHooks")
            .field("before_verify", &self.before_verify.len())
            .field("after_verify", &self.after_verify.len())
            .field("on_verify_failure", &self.on_verify_failure.len())
            .field("before_settle", &self.before_settle.len())
            .field("after_settle", &self.after_settle.len())
            .field("on_settle_failure", &self.on_settle_failure.len())
            .finish()
    }
}

impl SchemeHandlerHooks {
    /// Returns `true` if no hooks are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.before_verify.is_empty()
            && self.after_verify.is_empty()
            && self.on_verify_failure.is_empty()
            && self.before_settle.is_empty()
            && self.after_settle.is_empty()
            && self.on_settle_failure.is_empty()
    }

    /// Registers a hook to execute before payment verification.
    #[must_use]
    pub fn on_before_verify<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(FacilitatorVerifyContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<FacilitatorBeforeHookResult>, String>> + Send + 'static,
    {
        self.before_verify
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute after successful payment verification.
    #[must_use]
    pub fn on_after_verify<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(FacilitatorVerifyResultContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.after_verify
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute when payment verification fails.
    #[must_use]
    pub fn on_verify_failure<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(FacilitatorVerifyFailureContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<FacilitatorVerifyFailureHookResult>, String>>
            + Send
            + 'static,
    {
        self.on_verify_failure
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute before payment settlement.
    #[must_use]
    pub fn on_before_settle<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(FacilitatorSettleContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<FacilitatorBeforeHookResult>, String>> + Send + 'static,
    {
        self.before_settle
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute after successful payment settlement.
    #[must_use]
    pub fn on_after_settle<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(FacilitatorSettleResultContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.after_settle
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute when payment settlement fails.
    #[must_use]
    pub fn on_settle_failure<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(FacilitatorSettleFailureContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<FacilitatorSettleFailureHookResult>, String>>
            + Send
            + 'static,
    {
        self.on_settle_failure
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }
}

/// A [`SchemeHandler`] decorator that executes [`SchemeHandlerHooks`] around
/// an inner handler's verify and settle operations.
///
/// This is the primary integration point: wrap any scheme handler with hooks
/// to add observability, access control, or error recovery at the facilitator
/// layer.
///
/// # Example
///
/// ```ignore
/// let hooks = SchemeHandlerHooks::default()
///     .on_before_verify(|ctx| async move { Ok(None) });
/// let hooked = HookedSchemeHandler::new(inner_handler, hooks);
/// ```
pub struct HookedSchemeHandler {
    inner: Box<dyn SchemeHandler>,
    hooks: Arc<SchemeHandlerHooks>,
}

impl std::fmt::Debug for HookedSchemeHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookedSchemeHandler")
            .field("hooks", &self.hooks)
            .finish_non_exhaustive()
    }
}

impl HookedSchemeHandler {
    /// Wraps an inner handler with the given hooks.
    #[must_use]
    pub fn new(inner: Box<dyn SchemeHandler>, hooks: SchemeHandlerHooks) -> Self {
        Self {
            inner,
            hooks: Arc::new(hooks),
        }
    }
}

impl SchemeHandler for HookedSchemeHandler {
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, SchemeHandlerError>> + Send + '_>>
    {
        let hooks = Arc::clone(&self.hooks);
        let hook_ctx = FacilitatorVerifyContext {
            request: request.clone(),
        };

        Box::pin(async move {
            // Execute before-verify hooks
            for hook in &hooks.before_verify {
                if let Ok(Some(result)) = hook(hook_ctx.clone()).await
                    && result.abort
                {
                    return Err(SchemeHandlerError::OnchainFailure(result.reason));
                }
            }

            // Call the inner handler
            let inner_result = self.inner.verify(request).await;

            match inner_result {
                Ok(response) => {
                    // Execute after-verify hooks (errors logged, not propagated)
                    let result_ctx = FacilitatorVerifyResultContext {
                        ctx: hook_ctx,
                        response: response.clone(),
                    };
                    for hook in &hooks.after_verify {
                        let _ = hook(result_ctx.clone()).await;
                    }
                    Ok(response)
                }
                Err(err) => {
                    // Execute on-verify-failure hooks (may recover)
                    let failure_ctx = FacilitatorVerifyFailureContext {
                        ctx: hook_ctx,
                        error: err.to_string(),
                    };
                    for hook in &hooks.on_verify_failure {
                        if let Ok(Some(result)) = hook(failure_ctx.clone()).await
                            && result.recovered
                        {
                            return Ok(result.response);
                        }
                    }
                    Err(err)
                }
            }
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, SchemeHandlerError>> + Send + '_>>
    {
        let hooks = Arc::clone(&self.hooks);
        let hook_ctx = FacilitatorSettleContext {
            request: request.clone(),
        };

        Box::pin(async move {
            // Execute before-settle hooks
            for hook in &hooks.before_settle {
                if let Ok(Some(result)) = hook(hook_ctx.clone()).await
                    && result.abort
                {
                    return Err(SchemeHandlerError::OnchainFailure(result.reason));
                }
            }

            // Call the inner handler
            let inner_result = self.inner.settle(request).await;

            match inner_result {
                Ok(response) => {
                    // Execute after-settle hooks (errors logged, not propagated)
                    let result_ctx = FacilitatorSettleResultContext {
                        ctx: hook_ctx,
                        response: response.clone(),
                    };
                    for hook in &hooks.after_settle {
                        let _ = hook(result_ctx.clone()).await;
                    }
                    Ok(response)
                }
                Err(err) => {
                    // Execute on-settle-failure hooks (may recover)
                    let failure_ctx = FacilitatorSettleFailureContext {
                        ctx: hook_ctx,
                        error: err.to_string(),
                    };
                    for hook in &hooks.on_settle_failure {
                        if let Ok(Some(result)) = hook(failure_ctx.clone()).await
                            && result.recovered
                        {
                            return Ok(result.response);
                        }
                    }
                    Err(err)
                }
            }
        })
    }

    fn supported(
        &self,
    ) -> Pin<
        Box<dyn Future<Output = Result<proto::SupportedResponse, SchemeHandlerError>> + Send + '_>,
    > {
        self.inner.supported()
    }
}
