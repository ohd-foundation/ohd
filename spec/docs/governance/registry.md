# Standard Registry Governance

The `std.*` registry is centralized so every implementation uses one
canonical name for the same measurement. Without that, cross-source
aggregation breaks: `std.blood_glucose.value`, `std.glucose.value`, and
`std.cgm.glucose` would become separate facts even when they mean the same
thing.

## Proposal Process

New `std.*` event types and channels are added by pull request against the
canonical registry. A proposal must include:

- Sample data from the source or workflow the type is meant to represent.
- Event type name and registry namespace.
- Channel definitions, including paths, value types, required flags, enum
  values, and grouping.
- Units for every numeric channel.
- Sensitivity class for the event type and for channels that differ from the
  type default.
- Expected producers and consumers, such as manual entry, Health Connect,
  device bridge, lab import, or clinical system.
- Any aliases needed from prior experimental or vendor-specific names.

## Review Checklist

Reviewers check that the proposal:

- Fits an existing pattern before introducing a new shape.
- Uses canonical SI units or established clinical units already used by OHD.
- Does not overlap an existing `std.*` type or channel.
- Has the narrowest correct sensitivity classification.
- Keeps enum values append-only and stable.
- Uses clear channel paths that can survive multiple source systems.
- Includes enough sample data to test validation and import behavior.

## Versioning

The standard registry is append-only. New types, channels, enum values, and
aliases may be added. Existing IDs, paths, units, and enum ordinals are not
deleted or reused.

Deprecation happens through aliases and documentation, not removal. A renamed
type keeps the old name as an alias to the canonical type. A restructured
channel keeps an alias from the old path to the new channel. Storage resolves
aliases on insert and read; compaction may rewrite old rows later.

## Custom Namespaces

Vendor-specific or deployment-specific types that are not ready for standard
governance use `com.<vendor>.*` namespaces. These can ship independently and
round-trip through storage, export, and sync. If a custom type later becomes
standard, the registry adds aliases from the custom name to the new `std.*`
entry where appropriate.

## Runtime Lookup

Storage validates event type names, channel paths, value types, units,
required channels, and enum ordinals against the registry on insert. Unknown
`std.*` entries are rejected. Unknown `com.*` entries are accepted only if the
file has a matching custom registry row.

## Future Governance

The single canonical registry can later delegate review to federated groups:
country-specific medical terminology boards, specialty societies, or standards
bodies. Delegation changes who reviews proposals, not the storage rule that
one canonical name represents one measurement.
