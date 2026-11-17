"""LLDB summary providers for Yashy values.

Load from an LLDB session with:

    command script import crates/yashy-runtime/debug/pretty_printers.py

A provider is only used when the binary carries type information for the value.
Debug builds now emit a type table from HIR layouts, so a debug build can use
these; load them from an LLDB session with the command above, or point your
editor's LLDB integration at this file. The functions are pure: given a value
and its type name they return a string, never a memory address alone.
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


def array_summary(value, unused):
    """Render a sequence header as `[v1, v2, ...]`, bounded to a dozen items."""
    data = value.GetChildMemberWithName("data")
    length = value.GetChildMemberWithName("len")
    if not data.IsValid() or not length.IsValid():
        return ""
    try:
        count = int(length.GetValueAsSigned())
        if count < 0:
            return ""
        items = [data.GetChildAtIndex(i).GetValue() for i in range(min(count, 12))]
        more = ", ..." if count > 12 else ""
        return "[%s%s]" % (", ".join(items), more)
    except Exception:
        return ""


def map_summary(value, unused):
    """Render a map header by its entry count without exposing the table."""
    length = value.GetChildMemberWithName("count")
    if not length.IsValid():
        return ""
    return "map(%s entries)" % length.GetValue()


def __lldb_init_module(debugger, unused):
    for type_name, summary in (
        ("yashy_str", string_summary),
        ("Option", option_summary),
        ("Result", result_summary),
        ("Array", array_summary),
        ("Slice", array_summary),
        ("Map", map_summary),
    ):
        debugger.HandleCommand(
            "type summary add %s -F pretty_printers.%s" % (type_name, summary.__name__)
        )
