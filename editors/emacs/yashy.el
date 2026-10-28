;;; yashy.el --- Major mode for the Yashy language -*- lexical-binding: t; -*-
;;
;; A keyword added to the lexer must be added here too:
;; crates/yashy-lsp/tests/grammar.rs fails until it is.

;;; Code:

(defvar yashy-keywords
  '("fn" "struct" "enum" "const" "var" "let" "import" "extern" "unsafe" "defer"
    "return" "break" "continue" "if" "else" "for" "while" "in" "switch"
    "case" "default" "as" "move" "true" "false" "impl" "object" "pub"
    "requires" "ensures" "invariant")
  "Yashy keywords.")

(defvar yashy-types
  '("i64" "f64" "bool" "string" "void" "usize" "Option" "Result" "Channel" "map")
  "Yashy type names.")

(defvar yashy-font-lock-keywords
  `((,(regexp-opt yashy-keywords 'symbols) . font-lock-keyword-face)
    (,(regexp-opt yashy-types 'symbols) . font-lock-type-face)
    ("//.*$" . font-lock-comment-face)
    ("/\\*\\(?:.\\|\n\\)*?\\*/" . font-lock-comment-face)
    ("\"\\(?:\\\\.\\|[^\"\\\\]\\)*\"" . font-lock-string-face)
    ("\\_<[0-9]+\\.[0-9]+\\_>" . font-lock-constant-face)
    ("\\_<[0-9]+\\_>" . font-lock-constant-face)
    ("\\_<\\([A-Za-z_][A-Za-z0-9_]*\\)\\_>[ \t]*(" 1 font-lock-function-name-face))
  "Font-lock rules for `yashy-mode'.")

(defvar yashy-mode-syntax-table
  (let ((table (make-syntax-table)))
    ;; `//' starts a comment; `/* ... */' is handled by font-lock and by
    ;; `yashy-comment-dwim'.
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    (modify-syntax-entry ?_ "_" table)
    (modify-syntax-entry ?\" "\"" table)
    (modify-syntax-entry ?' "'" table)
    table)
  "Syntax table for `yashy-mode'.")

(defun yashy-comment-dwim (arg)
  "Comment or uncomment the region or line, in Yashy's line-comment style."
  (interactive "*P")
  (comment-dwim arg))

;;;###autoload
(define-derived-mode yashy-mode prog-mode "Yashy"
  "Major mode for editing Yashy source files."
  :syntax-table yashy-mode-syntax-table
  (setq-local comment-start "// ")
  (setq-local comment-end "")
  (setq-local comment-start-skip "//+\\s-*")
  (setq-local font-lock-defaults '(yashy-font-lock-keywords))
  (setq-local indent-tabs-mode nil)
  (setq-local tab-width 4))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.ya\\'" . yashy-mode))

(provide 'yashy)
;;; yashy.el ends here
