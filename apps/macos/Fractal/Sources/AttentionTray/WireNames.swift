import FractalClient

// FractalClient types whose names AttentionCore also uses. The `FractalClient` actor shadows
// its module, so these cannot be module-qualified where both modules are imported.

/// `GET v1/client/slo/history`'s value.
typealias WireSloHistory = SloHistory

/// `GET v1/client/availability/history`'s value.
typealias WireAvailabilityHistory = AvailabilityHistory
