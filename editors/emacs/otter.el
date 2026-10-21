;;; otter.el --- Major mode for the Otter language -*- lexical-binding: t; -*-
;;
;; A keyword added to the lexer must be added here too:
;; crates/otter-lsp/tests/grammar.rs fails until it is.

;;; Code:

(defvar otter-keywords
  '("fn" "struct" "enum" "const" "var" "let" "import" "extern" "unsafe" "defer"
    "return" "break" "continue" "if" "else" "for" "while" "in" "switch"
    "case" "default" "as" "move" "true" "false" "impl" "object")
  "Otter keywords.")

(defvar otter-types
  '("i64" "f64" "bool" "string" "void" "usize" "Option" "Result" "Channel" "map")
  "Otter type names.")

(defvar otter-font-lock-keywords
  `((,(regexp-opt otter-keywords 'symbols) . font-lock-keyword-face)
    (,(regexp-opt otter-types 'symbols) . font-lock-type-face)
    ("//.*$" . font-lock-comment-face)
    ("/\\*\\(?:.\\|\n\\)*?\\*/" . font-lock-comment-face)
    ("\"\\(?:\\\\.\\|[^\"\\\\]\\)*\"" . font-lock-string-face)
    ("\\_<[0-9]+\\.[0-9]+\\_>" . font-lock-constant-face)
    ("\\_<[0-9]+\\_>" . font-lock-constant-face)
    ("\\_<\\([A-Za-z_][A-Za-z0-9_]*\\)\\_>[ \t]*(" 1 font-lock-function-name-face))
  "Font-lock rules for `otter-mode'.")

(defvar otter-mode-syntax-table
  (let ((table (make-syntax-table)))
    ;; `//' starts a comment; `/* ... */' is handled by font-lock and by
    ;; `otter-comment-dwim'.
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    (modify-syntax-entry ?_ "_" table)
    (modify-syntax-entry ?\" "\"" table)
    (modify-syntax-entry ?' "'" table)
    table)
  "Syntax table for `otter-mode'.")

(defun otter-comment-dwim (arg)
  "Comment or uncomment the region or line, in Otter's line-comment style."
  (interactive "*P")
  (comment-dwim arg))

;;;###autoload
(define-derived-mode otter-mode prog-mode "Otter"
  "Major mode for editing Otter source files."
  :syntax-table otter-mode-syntax-table
  (setq-local comment-start "// ")
  (setq-local comment-end "")
  (setq-local comment-start-skip "//+\\s-*")
  (setq-local font-lock-defaults '(otter-font-lock-keywords))
  (setq-local indent-tabs-mode nil)
  (setq-local tab-width 4))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.ot\\'" . otter-mode))

(provide 'otter)
;;; otter.el ends here
