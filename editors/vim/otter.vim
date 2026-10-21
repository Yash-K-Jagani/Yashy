" Vim syntax file for the Otter language.
" Language: Otter
" A keyword added to the lexer must be added here too:
" crates/otter-lsp/tests/grammar.rs fails until it is.

if exists("b:current_syntax")
  finish
endif

syntax keyword otterKeyword fn struct enum const var let import extern unsafe defer return break continue if else for while in switch case default as move true false
syntax keyword otterType i64 f64 bool string void usize Option Result Channel map

syntax match otterComment "//.*$"
syntax region otterComment start="/\*" end="\*/" keepend
syntax region otterString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=otterEscape,otterInterpolation
syntax match otterEscape "\\." contained
syntax region otterInterpolation start="\${" end="}" contained
" Integers first, then floats, so the longer match does not lose its tail.
syntax match otterNumber "\v<\d+>"
syntax match otterNumber "\v<\d+\.\d+>"
syntax match otterFunction "\v<\a\w*>\ze\s*\("

highlight default link otterKeyword Keyword
highlight default link otterType Type
highlight default link otterComment Comment
highlight default link otterString String
highlight default link otterEscape SpecialChar
highlight default link otterInterpolation Special
highlight default link otterNumber Number
highlight default link otterFunction Function

let b:current_syntax = "otter"
