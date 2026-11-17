"""LLDB summary providers for Yashy values.

Load from an LLDB session with:

    command script import crates/yashy-runtime/debug/pretty_printers.py

A provider is only used when the binary carries type information for the value,
which the current backend does not emit yet. The file is committed so the
format is fixed before debug metadata lands; `yashyc debug` will load it
automatically once line and type tables exist. The functions are pure: given a
value and its type name they return a string, never a memory address alone.
"""


def _bytes(value, name):
    try:
        return bytes(value.GetChildMemberWithName(name).GetData()).decode("utf-8", "replace")
    except Exception:
        return ""


def string_summary(value, unused):
    """Render `yashy_str` as its text rather than as a pointer."""
    return repr(_bytes(value, "data"))


def option_summary(value, unused):
    """Render `Option[T]` as `some(v)` or `none`."""
    tag = value.GetChildMemberWithName("tag")
    if int(tag.GetValueAsSigned()) == 0:
        return "none"
    return "some(%s)" % value.GetChildMemberWithName("payload").GetValue()


def result_summary(value, unused):
    """Render `Result[T, E]` as `ok(v)` or `err(e)`."""
    tag = value.GetChildMemberWithName("tag")
    if int(tag.GetValueAsSigned()) == 0:
        return "ok(%s)" % value.GetChildMemberWithName("ok").GetValue()
    return "err(%s)" % value.GetChildMemberWithName("err").GetValue()


def __lldb_init_module(debugger, unused):
    for type_name, summary in (
        ("yashy_str", string_summary),
        ("Option", option_summary),
        ("Result", result_summary),
    ):
        debugger.HandleCommand(
            "type summary add %s -F pretty_printers.%s" % (type_name, summary.__name__)
        )
