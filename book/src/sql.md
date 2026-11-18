# SQL

`packages/db` separates SQL text from the values bound into it. A query is SQL
with `?` placeholders and a matching parameter list; a value is never spliced
into the statement. A quote or semicolon in a value is data, not syntax.

```yashy
import query;

fn find(int id) -> query.Query {
    let values = []string {};
    values.push(str_from_i64(id));
    const built = query.build("SELECT name FROM users WHERE id = ?", move values);
    switch built {
        case Ok(statement) { return statement; }
        case Err(_) { panic("the query is well formed"); }
    };
}
```

## Identifiers are values

A table or column is not a string. `query.table` and `query.column` accept only
a bare ASCII identifier — letters, digits and underscores, starting with a
letter — and answer `None` for a quote, semicolon, dot, space or leading digit.
`query.columns` validates a comma-separated list with the same grammar.

```yashy
import query;

fn listing() -> Option[query.Query] {
    const from = query.table("users");
    switch from {
        case None { return None; }
        case Some(table) {
            const names = query.columns("id, name");
            switch names {
                case None { return None; }
                case Some(list) {
                    const params = []string {};
                    const ordered = query.sort(
                        switch query.column("name") {
                            case Some(column) { column }
                            case None { return None; }
                        },
                        query.Order.Desc,
                    );
                    const built = query.select(table, list, "", move params, Some(ordered));
                    return switch built {
                        case Ok(statement) { Some(statement) }
                        case Err(_) { None }
                    };
                }
            };
        }
    };
}
```

## A closed ORDER BY

Dynamic ordering is a `query.Order` enum, not caller text. The only directions
are `Asc` and `Desc`, so a request cannot introduce an expression through the
order clause.

The package test carries an injection corpus: each shape that could carry
syntax is fed to all three constructors and must be refused. Real drivers and
wire codecs are separate packages; `query` stops at the typed statement.
