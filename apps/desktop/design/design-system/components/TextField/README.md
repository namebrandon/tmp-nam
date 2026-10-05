# TextField

A labelled single-line input with help or error text, and optional buttons to its right.

- `mono` for keys, hashes and paths.
- `error` as a string prints the message under the field: what's wrong and where to fix it ("Create a new one on tone3000.com › Settings › API").
- Children render to the right of the input: the field's own action ("Save Key", "Replace…").
- A saved secret shows masked and disabled, with Replace… and Remove next to it.
