//! Token input streams and tools converting to and from them..
//!
//! *“What’s up?” “I don’t know,” said Marvin, “I’ve never been there.”*
//!
//! [`Input`] is the primary trait used to feed input data into a chumsky parser. You can create them in a number of
//! ways: from strings, slices, arrays, etc.

pub use crate::stream::{BoxedStream, Stream};

use super::*;
#[cfg(feature = "memoization")]
use hashbrown::HashMap;

/// A trait for types that represents a stream of input tokens. Unlike [`Iterator`], this type
/// supports backtracking and a few other features required by the crate.
pub trait Input<'a>: 'a {
    /// The type used to keep track of the current location in the stream
    type Offset: Copy + Hash + Ord + Into<usize>;
    /// The type of singular items read from the stream
    type Token;
    /// The type of a span on this input - to provide custom span context see [`Spanned`] and [`WithContext`].
    type Span: Span;

    /// Get the offset representing the start of this stream
    fn start(&self) -> Self::Offset;

    /// Get the next offset from the provided one, and the next token if it exists
    ///
    /// Safety: `offset` must be generated by either `Input::start` or a previous call to this function.
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>);

    /// Create a span from a start and end offset.
    ///
    /// As with [`Input::next`], the offsets passed to this function must be generated by either [`Input::start`] or
    /// [`Input::next`].
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span;

    // Get the previous offset, saturating at zero
    #[doc(hidden)]
    fn prev(offs: Self::Offset) -> Self::Offset;

    /// Split an input that produces tokens of type `(T, S)` into one that produces tokens of type `T` and spans of
    /// type `S`.
    ///
    /// This is commonly required for lexers that generate token-span tuples. For example, `logos`'
    /// [`SpannedIter`](https://docs.rs/logos/0.12.0/logos/struct.Lexer.html#method.spanned) lexer generates such
    /// pairs.
    ///
    /// Also required is an 'End Of Input' (EoI) span. This span is arbitrary, but is used by the input to produce
    /// sensible spans that extend to the end of the input or are zero-width. Most implementations simply use some
    /// equivalent of `len..len` (i.e: a span where both the start and end offsets are set to the end of the input).
    /// However, what you choose for this span is up to you: but consider that the context, start, and end of the span
    /// will be recombined to create new spans as required by the parser.
    ///
    /// Although `Spanned` does implement [`BorrowInput`], please be aware that, as you might anticipate, the slices
    /// will be those of the original input (usually `&[(T, S)]`) and not `&[T]` so as to avoid the need to copy
    /// around sections of the input.
    fn spanned<T, S>(self, eoi: S) -> SpannedInput<T, S, Self>
    where
        Self: Input<'a, Token = (T, S)> + Sized,
        T: 'a,
        S: Span + Clone + 'a,
    {
        SpannedInput {
            input: self,
            eoi,
            phantom: PhantomData,
        }
    }

    /// Add extra context to spans generated by this input.
    ///
    /// This is useful if you wish to include extra context that applies to all spans emitted during a parse, such as
    /// an identifier that corresponds to the file the spans originated from.
    fn with_context<C>(self, context: C) -> WithContext<C, Self>
    where
        Self: Sized,
        C: Clone,
        Self::Span: Span<Context = ()>,
    {
        WithContext {
            input: self,
            context,
        }
    }
}

/// A trait for types that represent slice-like streams of input tokens.
pub trait SliceInput<'a>: Input<'a> {
    /// The unsized slice type of this input. For [`&str`] it's `&str`, and for [`&[T]`] it will be `&[T]`.
    type Slice;

    /// Get a slice from a start and end offset
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice;
    /// Get a slice from a start offset till the end of the input
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice;
}

// Implemented by inputs that reference a string slice and use byte indices as their offset.
/// A trait for types that represent string-like streams of input tokens
pub trait StrInput<'a, C: Char>:
    Input<'a, Offset = usize, Token = C> + SliceInput<'a, Slice = &'a C::Str>
{
}

/// Implemented by inputs that can have tokens borrowed from them.
pub trait BorrowInput<'a>: Input<'a> {
    /// Borrowed version of [`Input::next`] with the same safety requirements.
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>);
}

impl<'a> Input<'a> for &'a str {
    type Offset = usize;
    type Token = char;
    type Span = SimpleSpan<usize>;

    fn start(&self) -> Self::Offset {
        0
    }

    #[inline]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        if offset < self.len() {
            let c = unsafe {
                self.get_unchecked(offset..)
                    .chars()
                    .next()
                    .unwrap_unchecked()
            };
            (offset + c.len_utf8(), Some(c))
        } else {
            (offset, None)
        }
    }

    #[inline]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        range.into()
    }

    fn prev(offs: Self::Offset) -> Self::Offset {
        offs.saturating_sub(1)
    }
}

impl<'a> StrInput<'a, char> for &'a str {}

impl<'a> SliceInput<'a> for &'a str {
    type Slice = &'a str;

    #[inline]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        &self[range]
    }

    #[inline]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        &self[from]
    }
}

impl<'a, T: Clone> Input<'a> for &'a [T] {
    type Offset = usize;
    type Token = T;
    type Span = SimpleSpan<usize>;

    fn start(&self) -> Self::Offset {
        0
    }

    #[inline]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        let (offset, tok) = self.next_ref(offset);
        (offset, tok.cloned())
    }

    #[inline]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        range.into()
    }

    fn prev(offs: Self::Offset) -> Self::Offset {
        offs.saturating_sub(1)
    }
}

impl<'a> StrInput<'a, u8> for &'a [u8] {}

impl<'a, T: Clone> SliceInput<'a> for &'a [T] {
    type Slice = &'a [T];

    #[inline]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        &self[range]
    }

    #[inline]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        &self[from]
    }
}

impl<'a, T: Clone> BorrowInput<'a> for &'a [T] {
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        if let Some(tok) = self.get(offset) {
            (offset + 1, Some(tok))
        } else {
            (offset, None)
        }
    }
}

impl<'a, T: Clone + 'a, const N: usize> Input<'a> for &'a [T; N] {
    type Offset = usize;
    type Token = T;
    type Span = SimpleSpan<usize>;

    fn start(&self) -> Self::Offset {
        0
    }

    #[inline]
    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        let (offset, tok) = self.next_ref(offset);
        (offset, tok.cloned())
    }

    #[inline]
    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        range.into()
    }

    fn prev(offs: Self::Offset) -> Self::Offset {
        offs.saturating_sub(1)
    }
}

impl<'a, const N: usize> StrInput<'a, u8> for &'a [u8; N] {}

impl<'a, T: Clone + 'a, const N: usize> SliceInput<'a> for &'a [T; N] {
    type Slice = &'a [T];

    #[inline]
    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        &self[range]
    }

    #[inline]
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        &self[from]
    }
}

impl<'a, T: Clone + 'a, const N: usize> BorrowInput<'a> for &'a [T; N] {
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        if let Some(tok) = self.get(offset) {
            (offset + 1, Some(tok))
        } else {
            (offset, None)
        }
    }
}

/// A wrapper around an input that splits an input into spans and tokens. See [`Input::spanned`].
#[derive(Copy, Clone)]
pub struct SpannedInput<T, S, I> {
    input: I,
    eoi: S,
    phantom: PhantomData<T>,
}

impl<'a, T, S, I> Input<'a> for SpannedInput<T, S, I>
where
    I: Input<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    type Offset = I::Offset;
    type Token = T;
    type Span = S;

    fn start(&self) -> Self::Offset {
        self.input.start()
    }

    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        let (offs, tok) = self.input.next(offset);
        (offs, tok.map(|(tok, _)| tok))
    }

    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        let start = self
            .input
            .next(range.start)
            .1
            .map_or(self.eoi.start(), |(_, s)| s.start());
        let end = self
            .input
            .next(I::prev(range.end))
            .1
            .map_or(self.eoi.start(), |(_, s)| s.end());
        S::new(self.eoi.context(), start..end)
    }

    fn prev(offs: Self::Offset) -> Self::Offset {
        I::prev(offs)
    }
}

impl<'a, T, S, I> BorrowInput<'a> for SpannedInput<T, S, I>
where
    I: BorrowInput<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        let (offs, tok) = self.input.next_ref(offset);
        (offs, tok.map(|(tok, _)| tok))
    }
}

impl<'a, T, S, I> SliceInput<'a> for SpannedInput<T, S, I>
where
    I: SliceInput<'a, Token = (T, S)>,
    T: 'a,
    S: Span + Clone + 'a,
{
    type Slice = I::Slice;

    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice(&self.input, range)
    }
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice_from(&self.input, from)
    }
}

/// An input wrapper contains a user-defined context in its span, in addition to the span of the wrapped input. See
/// [`Input::with_context`].
#[derive(Copy, Clone)]
pub struct WithContext<Ctx, I> {
    input: I,
    context: Ctx,
}

impl<'a, Ctx: Clone + 'a, I: Input<'a>> Input<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    type Offset = I::Offset;
    type Token = I::Token;
    type Span = (Ctx, I::Span);

    fn start(&self) -> Self::Offset {
        self.input.start()
    }

    unsafe fn next(&self, offset: Self::Offset) -> (Self::Offset, Option<Self::Token>) {
        self.input.next(offset)
    }

    unsafe fn span(&self, range: Range<Self::Offset>) -> Self::Span {
        (self.context.clone(), self.input.span(range))
    }

    fn prev(offs: Self::Offset) -> Self::Offset {
        I::prev(offs)
    }
}

impl<'a, Ctx: Clone + 'a, I: BorrowInput<'a>> BorrowInput<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    unsafe fn next_ref(&self, offset: Self::Offset) -> (Self::Offset, Option<&'a Self::Token>) {
        self.input.next_ref(offset)
    }
}

impl<'a, Ctx: Clone + 'a, I: SliceInput<'a>> SliceInput<'a> for WithContext<Ctx, I>
where
    I::Span: Span<Context = ()>,
{
    type Slice = I::Slice;

    fn slice(&self, range: Range<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice(&self.input, range)
    }
    fn slice_from(&self, from: RangeFrom<Self::Offset>) -> Self::Slice {
        <I as SliceInput>::slice_from(&self.input, from)
    }
}

impl<'a, Ctx, C, I> StrInput<'a, C> for WithContext<Ctx, I>
where
    I: StrInput<'a, C>,
    I::Span: Span<Context = ()>,
    Ctx: Clone + 'a,
    C: Char,
{
}

/// Represents the progress of a parser through the input
pub struct Marker<'a, I: Input<'a>> {
    pub(crate) offset: I::Offset,
    err_count: usize,
}

impl<'a, I: Input<'a>> Copy for Marker<'a, I> {}
impl<'a, I: Input<'a>> Clone for Marker<'a, I> {
    fn clone(&self) -> Self {
        *self
    }
}

pub(crate) struct Errors<E> {
    pub(crate) alt: Option<Located<E>>,
    pub(crate) secondary: Vec<E>,
}

impl<E> Default for Errors<E> {
    fn default() -> Self {
        Self {
            alt: None,
            secondary: Vec::new(),
        }
    }
}

/// Internal type representing an input as well as all the necessary context for parsing.
pub struct InputRef<'a, 'parse, I: Input<'a>, E: ParserExtra<'a, I>> {
    pub(crate) input: &'parse I,
    pub(crate) offset: I::Offset,
    pub(crate) errors: Errors<E::Error>,
    // TODO: Don't use a result, use something like `Cow` but that allows `E::State` to not be `Clone`
    pub(crate) state: &'parse mut E::State,
    // TODO: Don't use `Option`, this is only here because we need to temporarily remove it in `with_input`
    pub(crate) ctx: Option<E::Context>,
    #[cfg(feature = "memoization")]
    pub(crate) memos: HashMap<(I::Offset, usize), Option<Located<E::Error>>>,
}

impl<'a, 'parse, I: Input<'a>, E: ParserExtra<'a, I>> InputRef<'a, 'parse, I, E> {
    pub(crate) fn new(input: &'parse I, state: &'parse mut E::State) -> Self
    where
        E::Context: Default,
    {
        Self {
            offset: input.start(),
            input,
            state,
            ctx: Some(E::Context::default()),
            errors: Errors::default(),
            #[cfg(feature = "memoization")]
            memos: HashMap::default(),
        }
    }

    pub(crate) fn with_ctx<'sub_parse, C, O>(
        &'sub_parse mut self,
        new_ctx: C,
        f: impl FnOnce(&mut InputRef<'a, 'sub_parse, I, extra::Full<E::Error, E::State, C>>) -> O,
    ) -> O
    where
        'parse: 'sub_parse,
        C: 'a,
    {
        use core::mem;

        let mut new_inp = InputRef {
            input: self.input,
            offset: self.offset,
            state: self.state,
            ctx: Some(new_ctx),
            errors: mem::replace(&mut self.errors, Errors::default()),
            #[cfg(feature = "memoization")]
            memos: HashMap::default(), // TODO: Reuse memoisation state?
        };
        let res = f(&mut new_inp);
        self.offset = new_inp.offset;
        self.errors = new_inp.errors;
        res
    }

    pub(crate) fn with_input<'sub_parse, O>(
        &'sub_parse mut self,
        new_input: &'sub_parse I,
        f: impl FnOnce(&mut InputRef<'a, 'sub_parse, I, E>) -> O,
    ) -> O
    where
        'parse: 'sub_parse,
    {
        use core::mem;

        let mut new_inp = InputRef {
            offset: new_input.start(),
            input: new_input,
            state: self.state,
            ctx: self.ctx.take(),
            errors: mem::replace(&mut self.errors, Errors::default()),
            #[cfg(feature = "memoization")]
            memos: HashMap::default(), // TODO: Reuse memoisation state?
        };
        let res = f(&mut new_inp);
        self.errors = new_inp.errors;
        self.ctx = new_inp.ctx;
        res
    }

    /// Get the input offset that is currently being pointed to.
    #[inline]
    pub fn offset(&self) -> I::Offset {
        self.offset
    }

    /// Save off a [`Marker`] to the current position in the input
    #[inline]
    pub fn save(&self) -> Marker<'a, I> {
        Marker {
            offset: self.offset,
            err_count: self.errors.secondary.len(),
        }
    }

    /// Reset the input state to the provided [`Marker`]
    #[inline]
    pub fn rewind(&mut self, marker: Marker<'a, I>) {
        self.errors.secondary.truncate(marker.err_count);
        self.offset = marker.offset;
    }

    #[inline]
    pub(crate) fn state(&mut self) -> &mut E::State {
        self.state
    }

    #[inline]
    pub(crate) fn ctx(&self) -> &E::Context {
        self.ctx.as_ref().expect("no ctx?")
    }

    #[inline]
    pub(crate) fn skip_while<F: FnMut(&I::Token) -> bool>(&mut self, mut f: F) {
        let mut offs = self.offset;
        loop {
            // SAFETY: offset was generated by previous call to `Input::next`
            let (offset, token) = unsafe { self.input.next(offs) };
            if token.filter(&mut f).is_none() {
                self.offset = offs;
                break;
            } else {
                offs = offset;
            }
        }
    }

    #[inline]
    pub(crate) fn next(&mut self) -> (I::Offset, Option<I::Token>) {
        // SAFETY: offset was generated by previous call to `Input::next`
        let (offset, token) = unsafe { self.input.next(self.offset) };
        self.offset = offset;
        (self.offset, token)
    }

    #[inline]
    pub(crate) fn next_ref(&mut self) -> (I::Offset, Option<&'a I::Token>)
    where
        I: BorrowInput<'a>,
    {
        // SAFETY: offset was generated by previous call to `Input::next`
        let (offset, token) = unsafe { self.input.next_ref(self.offset) };
        self.offset = offset;
        (self.offset, token)
    }

    /// Get the next token in the input. Returns `None` for EOI
    pub fn next_token(&mut self) -> Option<I::Token> {
        self.next().1
    }

    /// Peek the next token in the input. Returns `None` for EOI
    pub fn peek(&self) -> Option<I::Token> {
        // SAFETY: offset was generated by previous call to `Input::next`
        unsafe { self.input.next(self.offset).1 }
    }

    /// Skip the next token in the input.
    #[inline]
    pub fn skip(&mut self) {
        let _ = self.next();
    }

    #[inline]
    pub(crate) fn slice(&self, range: Range<I::Offset>) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.input.slice(range)
    }

    #[allow(dead_code)]
    #[inline]
    pub(crate) fn slice_from(&self, from: RangeFrom<I::Offset>) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.input.slice_from(from)
    }

    #[cfg_attr(not(feature = "regex"), allow(dead_code))]
    #[inline]
    pub(crate) fn slice_trailing(&self) -> I::Slice
    where
        I: SliceInput<'a>,
    {
        self.input.slice_from(self.offset..)
    }

    /// Return the span from the provided [`Marker`] to the current position
    #[inline]
    pub unsafe fn span_since(&self, before: I::Offset) -> I::Span {
        self.input.span(before..self.offset)
    }

    #[inline]
    #[cfg(feature = "regex")]
    pub(crate) fn skip_bytes<C>(&mut self, skip: usize)
    where
        C: Char,
        I: StrInput<'a, C>,
    {
        self.offset += skip;
    }

    #[inline]
    pub(crate) fn emit(&mut self, error: E::Error) {
        self.errors.secondary.push(error);
    }

    #[inline]
    pub(crate) fn add_alt(&mut self, error: impl Into<Option<Located<E::Error>>>) {
        self.errors.alt = match (self.errors.alt.take(), error.into()) {
            (Some(a), Some(b)) => Some(a.prioritize(b, |a, b| a.merge(b))),
            (a, b) => a.or(b),
        };
    }

    pub(crate) fn into_errs(self) -> Vec<E::Error> {
        self.errors.secondary
    }
}

/// Struct used in [`Parser::validate`] to collect user-emitted errors
pub struct Emitter<E> {
    emitted: Vec<E>,
}

impl<E> Emitter<E> {
    pub(crate) fn new() -> Emitter<E> {
        Emitter {
            emitted: Vec::new(),
        }
    }

    pub(crate) fn errors(self) -> Vec<E> {
        self.emitted
    }

    /// Emit a non-fatal error
    pub fn emit(&mut self, err: E) {
        self.emitted.push(err)
    }
}
