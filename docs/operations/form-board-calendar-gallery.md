# Forms, boards, calendars, and galleries

These are four kinds of saved view. An app page of kind `View` that names one opens it.

## Form

A form view is some fields of a table, in an order. Its config:

```json
{
  "form_fields": [
    { "name": "kind", "label": "What kind of help?" },
    { "name": "amount", "label": "How much?", "help": "In dollars",
      "show_when": [{ "field": "kind", "op": "eq", "value": "grant" }] }
  ],
  "success_message": "Thank you. We have your request."
}
```

A field shows only when every condition of its `show_when` holds. The operators are `eq`, `ne`, `in`, `is_empty` and `is_not_empty`. A condition names a field above it on the form.

- An answer is empty when it is missing, null, blank, or an empty list.
- A hidden field answers as empty to the fields after it.
- `in` takes a list, or a comma list in a string. A list answer matches `eq` and `in` when any item does.

Both the browser and the server run the conditions. The server is authoritative, and it decides at submit:

- A field the conditions hid is not required, even when the table requires it.
- An answer for a hidden field is refused with `400`, so a client that skipped a condition is told.
- An answer for a field that is not on the form is refused.
- Everything else goes through the ordinary create path. The table's rules, the person's rights and the app's guards all apply, and a person who may not write gets `403`.

The browser's evaluator is tested against vectors that the Rust evaluator generates into `apps/web/src/test/vectors/show-when.json`. After a change to `show_when.rs`, run `SCAFFOLDRY_WRITE_VECTORS=1 cargo test -p scaffoldry-engine --test show_when_test`.

Submit is `POST /apps/{slug}/views/{id}/submit` with `{ "data": { ... } }`.

## Board

A board is a view of kind `kanban` with `kanban_column_field` set to a single select. There is one column for each option.

- The server groups the rows, so a column's count is the aggregate's count.
- A column opens with 50 cards. `Load more` fetches 50 more by cursor.
- A move updates the record with the version the card was read at. If the record changed, the move is refused and the card goes back.
- With a card focused, Alt and the left or right arrow move it to the next column. The move is announced: `Moved to Offer made.`
- `column_limits` is optional, such as `{ "applied": 20 }`. A column over its limit shows a warning. It does not block a move.

## Calendar

A calendar is a view of kind `calendar` with `calendar_date_field` set to a date field. It shows a month, a week or a day.

- Each range is one request, `GET .../data?from=2026-03-01&to=2026-03-31`, both ends included. The date field is indexed by the view.
- The server answers with at most 500 events, earliest first, and `calendar: { shown, total, more }`. The screen shows `+1,500 more`.
- An event opens the record.

## Gallery

A gallery is a view of kind `gallery`. `gallery_cover_field` names an attachment field, and `gallery_card_fields` lists fields to show on each card.

- The cover is the first image of the attachment field. The file is read through the same record and field checks as any file.
- A record with no image, or one whose image cannot be read, shows the initials of its name.
