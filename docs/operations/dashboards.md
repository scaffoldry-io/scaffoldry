# Dashboards

A dashboard is a view of kind `dashboard`. It lays widgets out on a grid that is 12 columns wide. A dashboard is presentation, so saving one needs no proposal.

## What a widget shows

| Body | Shows |
| --- | --- |
| `view` | The rows of a saved view, in a table. A row can be selected |
| `chart` | A bar chart of a summary view, with a table alternative |
| `stat` | One number: a count, sum, average, minimum, maximum or distinct count of a table |
| `text` | Markdown, drawn by the safe renderer |
| `page` | A custom page that the app's manifest already approved |

A `page` widget cannot name a page that is not in the manifest. Each page is still governed by its own approval.

## Rules a save must meet

- Every widget has a unique id and a title.
- A widget lies inside the 12 columns, and its width and height are at least 1.
- No two widgets overlap.
- A dashboard holds at most 30 widgets.
- A `view` or `chart` widget names a view of this app. A chart names a summary view. A shared dashboard may not name a personal view.
- A link names another widget of the same dashboard, and that widget is a `view` widget.

A save that breaks one of these is `400`, and the message names the widget.

## Linked widgets

A widget may carry `select_by: { source_widget, source_field, target_field }`. When a person selects a row in the source widget, the page asks the server for this widget again with `select=<the row's source_field value>`. The server adds `target_field = <value>` to the widget's own filter. Clearing the selection sends no `select`.

The browser filters nothing. The widget still goes through the person's row rules and field rules, so a selection cannot reach a row or a field the person may not read. A selection on a hidden `target_field` is refused like any other filter on it.

On a table over 10,000 rows, `target_field` must be an indexed field. The dashboard does not build the index. A view that filters or sorts by the field does.

## Pages in a widget

A page in a widget is told which record is selected in the widget it is linked to. That is one id. A page reads it as `scaffoldry.context.selection`, and `scaffoldry.onContextChange(fn)` calls `fn` when it changes. The value is a string of at most 128 characters, or `null`. A page cannot set it. It gives the page nothing it could not already read.

## Changing the layout

Open the dashboard and press `Edit layout`. Focus a widget and use the arrow keys to move it. Hold Shift with the arrow keys to resize it. Four numbers, X, Y, Width and Height, set the exact values. A move that would leave the grid or overlap another widget is refused, and the message says which.

Below 900 px wide the widgets stack, in reading order: top to bottom, then left to right.

A dashboard opens from an app page of kind `View` that names it.
