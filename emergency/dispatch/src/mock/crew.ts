// Mock crew roster.
//
// The crew roster is operator-side state (the EMS station's IdP knows
// who's on shift). Not on the OHDC wire today; the relay's roster sync
// endpoint is the long-term source. v0 ships mock data so the page
// can be reviewed visually.

import type { CrewMember } from "../types";

const NOW = Date.now();

export const MOCK_CREW: CrewMember[] = [
  {
    responder_label: "horak.p",
    display_name: "P. Horak",
    on_duty: true,
    on_duty_since_ms: NOW - 4 * 3600_000,
    current_case_ulid: "01JD9X8MZQK3F0VTNPB7WAYG2H",
    last_seen_ms: NOW - 30_000,
    contact: "+420 777 111 001",
  },
  {
    responder_label: "dvorak.m",
    display_name: "M. Dvorak",
    on_duty: true,
    on_duty_since_ms: NOW - 5 * 3600_000,
    current_case_ulid: "01JD9X7Y1QV3FE0RA20BKDMHWS",
    last_seen_ms: NOW - 90_000,
    contact: "+420 777 111 002",
  },
  {
    responder_label: "svoboda.j",
    display_name: "J. Svoboda",
    on_duty: true,
    on_duty_since_ms: NOW - 3 * 3600_000,
    current_case_ulid: "01JD9X3K8RW1ZAPB9TGXH4NMVF",
    last_seen_ms: NOW - 4 * 60_000,
    contact: "+420 777 111 003",
  },
  {
    responder_label: "kralova.i",
    display_name: "I. Kralova",
    on_duty: true,
    on_duty_since_ms: NOW - 2 * 3600_000,
    current_case_ulid: null,
    last_seen_ms: NOW - 8 * 60_000,
    contact: "+420 777 111 004",
  },
];
