//! Which import a document reads next. Disk reads block, so a document runs one reader at a
//! time and keeps only the latest request: a superseded read cannot install a stale table or
//! overlap a newer decode, and a failed request keeps the table that is shown.

/// The options a document reads its file with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Import {
    pub sheet: Option<String>,
    pub has_header: bool,
}

pub(super) enum Load {
    Ready,
    Pending(Import),
    Failed(Import, gewu_core::Error),
}

/// What to do with a finished read.
pub(super) enum ReadOutcome<T> {
    /// The read is the latest request: install its table.
    Install(T),
    /// The latest request failed; the shown table, if any, stays.
    Failed,
    /// A newer request arrived meanwhile: start a reader for it.
    Next,
    /// The request was withdrawn meanwhile: nothing to read.
    Withdrawn,
}

pub(super) struct ImportSession {
    /// The import of the shown table, or the first request before any table is shown.
    displayed: Import,
    has_table: bool,
    load: Load,
    /// A reader was started and has not finished.
    reading: bool,
}

impl ImportSession {
    /// A document's first request. A reader should start.
    pub(super) fn new(initial: Import) -> Self {
        Self {
            displayed: initial.clone(),
            has_table: false,
            load: Load::Pending(initial),
            reading: true,
        }
    }

    pub(super) fn load(&self) -> &Load {
        &self.load
    }

    #[cfg(test)]
    pub(super) fn displayed(&self) -> &Import {
        &self.displayed
    }

    /// The import the controls show as chosen: the pending request, a failed request while
    /// no table is shown, or else the shown table's.
    pub(super) fn selection(&self) -> &Import {
        match &self.load {
            Load::Pending(import) => import,
            Load::Failed(import, _) if !self.has_table => import,
            _ => &self.displayed,
        }
    }

    /// Request `import`. Returns whether a reader should start; one already reading picks
    /// up the latest request when it finishes.
    pub(super) fn request(&mut self, import: Import) -> bool {
        if self.has_table && self.displayed == import {
            self.load = Load::Ready;
            return false;
        }
        self.load = Load::Pending(import);
        !std::mem::replace(&mut self.reading, true)
    }

    /// A reader starts: the request it reads, or `None` when nothing is pending any more.
    /// Several requests may arrive before the reader runs; it reads the latest.
    pub(super) fn begin_read(&mut self) -> Option<Import> {
        let Load::Pending(import) = &self.load else {
            self.reading = false;
            return None;
        };
        self.reading = true;
        Some(import.clone())
    }

    pub(super) fn read_finished<T>(
        &mut self,
        import: Import,
        result: gewu_core::Result<T>,
    ) -> ReadOutcome<T> {
        match &self.load {
            Load::Pending(latest) if *latest == import => {}
            Load::Pending(_) => return ReadOutcome::Next,
            _ => {
                self.reading = false;
                return ReadOutcome::Withdrawn;
            }
        }
        self.reading = false;
        match result {
            Ok(table) => ReadOutcome::Install(table),
            Err(error) => {
                self.load = Load::Failed(import, error);
                ReadOutcome::Failed
            }
        }
    }

    /// A table read with `import` is shown. Its sheet may be resolved from the request.
    pub(super) fn installed(&mut self, import: Import) {
        self.displayed = import;
        self.has_table = true;
        self.load = Load::Ready;
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
