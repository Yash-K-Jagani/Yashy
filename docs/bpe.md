# BPE audit — Phase 4 spellings

This is the **language-side** half of the Phase 4 BPE goal. It measures the
Yashy lexer's own tokenisation; it does not claim anything about the external
tokenizers the design names.

## What is measured

`cargo test -p yashy-driver --test bpe` lexes every spelling Phase 4 added and
asserts the token count:

| Spelling | Yashy tokens |
|---|---|
| `trait`, `type`, `derive`, `unsafe`, `noalias`, `packed`, `align`, `transparent`, `matches`, `in`, `is`, `and`, `or`, `not`, `@` | 1 |
| `const fn`, `let else` | 2, by design |

A spelling that split into unexpected tokens would fail that test. The
formatter and the editor grammars read the same token classes, so one token
here means one token in those surfaces too.

## What is not measured

The design asks that each new keyword and operator be **one token** in
`o200k`, `cl100k` and a Llama-family tokenizer, and that the corpus reach 97%
single-token lexical tokens. Those vocabularies are not vendored in this
repository, so no honest measurement exists here. `yashyc tokens --bpe`
reports the compiler's own lexer side and says so in its output.

## Follow-up

Vendor the three vocabularies (Phase 2 Task 8 owes them), then extend this page
with the per-spelling counts from `yashyc tokens --bpe` and a real corpus row.
Until then the external half stays open, and Phase 13's BPE gate must not treat
this page as evidence for it.
