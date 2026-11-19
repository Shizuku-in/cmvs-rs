//! Bridges asynchronous platform text controls to the canvas comment editor.
use cmvs_core::{
    TextEncoding,
    host::platform::{TextDialogRequest, TextDialogResult},
};
use cmvs_font::text_codec;
use std::error::Error;

enum State {
    Requested(CommentInputRequest),
    Waiting,
    Answered(Option<String>),
}

pub struct CommentInputRequest {
    pub id: u32,
    pub initial: String,
    pub max_chars: u32,
}

#[derive(Default)]
pub struct CommentInput {
    state: Option<State>,
    id: u32,
}

impl CommentInput {
    pub fn poll(
        &mut self,
        request: &TextDialogRequest,
        encoding: TextEncoding,
    ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
        match self.state.take() {
            None => {
                self.id = self.id.wrapping_add(1);
                self.state = Some(State::Requested(CommentInputRequest {
                    id: self.id,
                    initial: text_codec::decode(encoding, &request.initial)?,
                    max_chars: request.character_limit(encoding),
                }))
            }
            Some(State::Answered(None)) => return Ok(TextDialogResult::Cancelled),
            Some(State::Answered(Some(text))) => {
                if text.len() > 1024 || text.chars().any(char::is_control) {
                    return Err("invalid single-line comment text".into());
                }
                return Ok(TextDialogResult::Accepted(text_codec::encode(
                    encoding, &text,
                )?));
            }
            state => self.state = state,
        }
        Ok(TextDialogResult::Pending)
    }

    /// Take each request once, after which a single answer can be submitted.
    pub fn take_request(&mut self) -> Option<CommentInputRequest> {
        if !matches!(self.state, Some(State::Requested(_))) {
            return None;
        }
        match self.state.replace(State::Waiting) {
            Some(State::Requested(request)) => Some(request),
            _ => unreachable!(),
        }
    }

    pub fn submit(&mut self, id: u32, text: Option<String>) {
        if self.waiting_id() == Some(id) {
            self.state = Some(State::Answered(text));
        }
    }

    pub fn waiting_id(&self) -> Option<u32> {
        matches!(self.state, Some(State::Waiting)).then_some(self.id)
    }

    pub fn cancel(&mut self) {
        self.state = None;
    }
}
