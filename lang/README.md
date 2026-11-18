# Languages

What PimpMyShell says lives here, one file per language, named for the language's code: `en_US.toml`, `en_RN.toml`.

`en_US.toml` is the source. It says which texts there are, and every other file is a translation of it. The files are
compiled into the programs (the `pms-lang` crate), so nothing is read from disk when PimpMyShell runs, and a language
is there for everybody as soon as its file is.

## Adding a language

1. Copy `en_US.toml` to a file named for your language's code, as Windows spells it but with an underline: `de_DE.toml`,
   `pt_BR.toml`.
2. Under `[language]`, set `name` to what the language calls itself (`Deutsch`), and `windows` to how Windows' names
   for it begin (`de`), so that somebody whose Windows is in it gets it without asking. Leave `windows` out for a
   language that should only ever be chosen.
3. Translate what's in the quotes. Leave out any text you haven't a translation for yet: it's said in English until
   you do.
4. `cargo xtask lang` says how far along each language is, and which texts it hasn't got.

## The rules

- A text is `name = "what it says"`, on one line. Don't change the names, or the `[sections]`.
- `{braces}` hold what's filled in when the text is said, like a number or a file's name. Keep them, with the same
  word inside, wherever in your sentence they belong.
- `\"` is a quote inside a text, and `\n` a new line. To have a brace as a brace, double it: `{{`.
- A name or a placeholder English hasn't stops the build, and says which file and line.

Trademarks, licenses and anything else with legal weight are better left as English says them: leave those texts out.

## Codes

A code is a language and a country, as in `en_US`. One here isn't anybody's country: `en_RN` is Redneck, and the code
is Red Hat's own. In 1998 Red Hat Linux came with a Redneck translation of its setup tools, by Donnie Barnes, under
that name.
