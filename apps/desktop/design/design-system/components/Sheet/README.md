# Sheet

A window-modal sheet that drops from the toolbar edge over a scrim, with a title, body and a button row.

- Uses: checking files before sending, confirming a removal, confirming the card erase, allowed variants.
- Title is the question or the action ("Erase this card and create the NAM card?"). Body: what is affected, what can't be undone.
- `actions` right-aligned, Cancel first, the action last. Destructive confirmations: `alert`, the danger button, focus on Cancel.
- `extra` adds a left-aligned plain button ("Add More Files…", "Restore Default").
- The scrim is absolutely positioned: the window element must be `position: relative`. Long operations never continue inside a sheet.
