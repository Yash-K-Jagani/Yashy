# json-report

Reads JSON-ish objects from standard input, one per line, and prints a JSON
summary with the number of documents and distinct keys. It uses `JSON.quote`
from the standard library for the note field.

```text
printf '{"name": "yashyc"}\n{"age": 3}\n{"name": "again"}\n' | yashyc run main.ya
# {"documents": 3, "distinct_keys": 2, "note": "counted by Yashy"}
yashyc test
```
