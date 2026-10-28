# Printing

File ▸ Print… prints the frames of the current page (or the selected frames) on paper. The command is
`file.print`; like every command that needs a person it only opens the dialog: programmatic callers (CLI,
MCP, the control channel) never print.

## What is printed

One sheet per top-level visible frame, in reading order. The engine lays them out
(`figcraft_engine::cmd::file::print_plan`) as a print-ready PDF written by `figcraft-pdf`:

- **Paper**: US Letter, US Legal, Tabloid, A3, A4, A5, or *Frame size* (each sheet is its frame's size and
  the printer decides how it fits). The default is A4.
- **Orientation**: automatic (a frame wider than tall turns the sheet to landscape), portrait or landscape.
- **Scale**: *Actual size, shrunk to fit when larger* (1 px = 1 pt; a frame is never enlarged) or *Fit to the
  page* (scaled up or down to fill the printable area). Sheets are centred with a quarter inch margin.
- Bleed, marks and CMYK are the Export dialog's PDF print options, not print-dialog settings.

`figcraft_pdf::Fit` is the layout (`PdfOptions::fit`); bleed/marks ignore it because they size the page
themselves.

## Where it goes

One **Printer** list: every queue of this computer (the default first, marked), then the network printers
queried in this session (by the name each reported, kept for the session only, never saved), then
**Network printer…**, which shows the address field below. Refresh lists the queues again.

- **A queue of this computer**: the spooler (Windows spooler, CUPS) through craft-print. Copies, two-sided and
  Grayscale are craft-print `Job` settings. The job asks for the paper exactly only when the person picked a
  paper in the dialog; otherwise (frame size, or the opening A4) it goes on the queue's own paper, shrunk to fit
  (`Scaling::ShrinkToFit`), so a queue without a 13.33 x 7.50 in paper does not refuse it. Closing the app
  gives a hand-over still in progress up to 25 s (`SHUTDOWN_DEADLINE`) to finish; after that the app exits
  anyway and logs a warning that the job may or may not have reached the queue (check the print queue), so a
  stalled driver or spooler cannot leave a window-less process behind.
- **Grayscale** (the queue's checkbox, or the network printer's Color setting) is honest: the PDF is written gray
  (`PdfOptions::grayscale`, Rec. 709 weights; the raster path uses the same weights) and the driver is also
  asked for monochrome (`Job::grayscale`), so a driver that ignores the hint still prints gray.
- **A network printer** (desktop builds with the default `network-print` feature): the address of a WS-Print
  or IPP printer. `ipp://…` or an `http://` address with an `/ipp/` path segment is IPP; anything else is
  WS-Print. Only numeric private, link-local or loopback addresses are accepted (craft-wsd's policy), no host
  names and no TLS. Nothing connects until **Query printer** is pressed, and a job needs the **confirmation
  box** ticked for that job.
  - Every option (two-sided, orientation, quality, colour, resolution, copies) has three states: supported
    (offered from what the printer listed), unsupported (shown disabled with the reason) or unknown (enabled
    with "Not confirmed by the printer"; the value goes as a hint).
  - A printer that lists PDF gets the PDF as it is. One that lists only PWG raster gets the same frames
    rendered with FigCraft's renderer (`netprint/raster.rs`) at the resolution it lists nearest 300 dpi. One
    that lists neither blocks Print.
  - The job names a paper (when the printer lists it) only when the person picked one, or, with *Frame
    size*, when every sheet is the same paper of the dialog either way up; the opening A4 and mixed sizes
    name none, so the printer uses its own, as a queue of this computer does.
  - One press of Print is one tracked craft-print `PrintJob`: accepted, followed to its end, retried only by
    the person (Retry), reconciled against the printer's job lists (reads only) when the outcome is unknown.
    Every status line names the printer by what it reported about itself, never by its address.

The web build has no spooler and no sockets: File ▸ Print says to export the frames to PDF.

## Test switches

- `PDFCRAFT_TEST_EVENTS_BIND` (test builds): `ip:port` for the opt-in WS-Eventing listener a WS-Print printer
  can call; without it jobs are polled.

## Code map

| | |
|---|---|
| `crates/pdf` | `Fit`, `FitOrientation`, `FitScale`, `PdfOptions::fit` |
| `crates/engine/src/cmd/file.rs` | `print_plan`, `printable_frames`, `file.print` |
| `crates/ui-egui/src/print_ui.rs` | the dialog, the spooler workers |
| `crates/ui-egui/src/network_glue.rs` (+ `tracked`, `caps`) | craft-ui `printing::network` host: workers, wording, three-state caps, tracked status lines |
| `crates/ui-egui/src/netprint.rs` (+ `station`, `raster`) | craft-print layer: `Station` (WS-Print and IPP behind one registry), `FramesToRaster` |
