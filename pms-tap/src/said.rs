//! What the TAP says, in the language the daemon tells it (`lang <code>`, at every apply and when the settings change it): the
//! words are pms-lang's `[tap]`, compiled in. Only that section is used here, so only its texts (in every language) are linked
//! into the TAP; the rest of pms-lang's tables, the settings' and the daemon's, aren't, which matters for a DLL that Explorer
//! keeps a copy of for good.
//!
//! Words built into something that stays up (a remote's keys and headings) are kept here with the text each says, and said again
//! when the language has changed since: the XAML stays as it is, and only its words change.

use std::cell::Cell;

use pms_xaml::TextBlock;
use windows_core::HSTRING;

pub(crate) use pms_lang::tap as t;

use crate::commands::R;

/// `lang <code>`: from now on, the TAP says everything in that language. A code it doesn't know changes nothing.
pub(crate) fn command(args: &[String]) -> R<String> {
    let code = args.first().ok_or("missing <code>")?;
    let at = pms_lang::find(code).ok_or_else(|| format!("no language `{code}` in this build"))?;
    pms_lang::set(at);
    // What's up now says it the new way when it next looks at itself; the meters' mic label is said again at once.
    crate::meters::on_ui(crate::meters::retell);
    Ok(format!("lang {}", pms_lang::language().code))
}

/// Words built in, each with the text it says, and the language they were last said in.
pub(crate) struct Said {
    texts: Vec<(TextBlock, t::Id)>,
    language: Cell<usize>,
}

impl Default for Said {
    fn default() -> Said {
        Said { texts: Vec::new(), language: Cell::new(pms_lang::current()) }
    }
}

impl Said {
    /// `block`, saying `text` from now on.
    pub(crate) fn keep(&mut self, block: &TextBlock, text: t::Id) -> R<()> {
        block.SetText(&HSTRING::from(text.t())).map_err(|e| e.to_string())?;
        self.texts.push((block.clone(), text));
        Ok(())
    }

    /// Whether the language has changed since they were said; if it has, they're said again, and this is true.
    pub(crate) fn follow(&self) -> R<bool> {
        let now = pms_lang::current();
        if self.language.replace(now) == now {
            return Ok(false);
        }
        for (block, text) in &self.texts {
            block.SetText(&HSTRING::from(text.t())).map_err(|e| e.to_string())?;
        }
        Ok(true)
    }
}
