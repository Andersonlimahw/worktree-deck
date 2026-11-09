use std::fmt;
use std::process::ExitCode;

/// Stable failure categories. The string form is the JSON `error.code` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    GitFailed,
    InvalidGitOutput,
    Usage,
    NotARepository,
    Refused,
    NotConfirmed,
    NotFound,
    Io,
}

impl ErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::GitFailed => "git_failed",
            Self::InvalidGitOutput => "invalid_git_output",
            Self::Usage => "usage",
            Self::NotARepository => "not_a_repository",
            Self::Refused => "refused",
            Self::NotConfirmed => "not_confirmed",
            Self::NotFound => "not_found",
            Self::Io => "io",
        }
    }

    pub(crate) fn exit_status(self) -> u8 {
        match self {
            Self::GitFailed | Self::InvalidGitOutput | Self::Io => 1,
            Self::Usage => 2,
            Self::NotARepository => 3,
            Self::Refused | Self::NotConfirmed => 4,
            Self::NotFound => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppError {
    pub(crate) code: ErrorCode,
    pub(crate) message: String,
}

impl AppError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message)
    }

    pub(crate) fn git_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::GitFailed, message)
    }

    pub(crate) fn invalid_git(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidGitOutput, message)
    }

    pub(crate) fn not_a_repository() -> Self {
        Self::new(
            ErrorCode::NotARepository,
            "the current directory is not inside a git repository",
        )
    }

    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Refused, message)
    }

    pub(crate) fn not_confirmed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotConfirmed, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Io, message)
    }

    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code.exit_status())
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}
