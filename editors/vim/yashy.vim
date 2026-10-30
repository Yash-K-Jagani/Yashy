" Vim syntax file for the Yashy language.
" Language: Yashy
" A keyword added to the lexer must be added here too:
" crates/yashy-lsp/tests/grammar.rs fails until it is.

if exists("b:current_syntax")
  finish
endif

syntax keyword yashyKeyword fn struct enum const var let import extern unsafe pure io defer return break continue if else for while in switch case default as move true false impl object pub requires ensures invariant
syntax keyword yashyType i64 f64 bool string void usize Option Result Channel map

syntax match yashyComment "//.*$"
syntax region yashyComment start="/\*" end="\*/" keepend
syntax region yashyString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=yashyEscape,yashyInterpolation
syntax match yashyEscape "\\." contained
syntax region yashyInterpolation start="\${" end="}" contained
" Integers first, then floats, so the longer match does not lose its tail.
syntax match yashyNumber "\v<\d+>"
syntax match yashyNumber "\v<\d+\.\d+>"
syntax match yashyFunction "\v<\a\w*>\ze\s*\("

highlight default link yashyKeyword Keyword
highlight default link yashyType Type
highlight default link yashyComment Comment
highlight default link yashyString String
highlight default link yashyEscape SpecialChar
highlight default link yashyInterpolation Special
highlight default link yashyNumber Number
highlight default link yashyFunction Function

let b:current_syntax = "yashy"
