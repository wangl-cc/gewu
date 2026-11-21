## Summary

<!-- What changes and why. Link the issue or discussion if there is one. -->

## Review

<!-- Tick what applies. The review checklist lists the rules and what guards each one. -->

- [ ] Layers touched: Core / FFI / GewuKit / App. Each change sits in the layer that owns it ([architecture](https://github.com/wangl-cc/gewu/blob/main/docs/architecture.md#layers)).
- [ ] Rules in the [review checklist](https://github.com/wangl-cc/gewu/blob/main/docs/review-checklist.md) for those layers still hold; rules guarded only by review are called out below.
- [ ] `unsafe`, the C data plane or lent memory: lifetimes re-checked.
- [ ] New or changed failures keep structured kinds through to `ErrorPresentation`.
- [ ] Architecture, decision records and READMEs match the code. A new decision has a record.

## Verification

<!-- Say which ran: build, automated tests (which), UI checked in the running app, performance measured. -->

<!-- markdownlint-disable-file MD041 -->
