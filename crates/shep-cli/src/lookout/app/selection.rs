//! Which rows are visible, and where the cursor sits among them.
//!
//! Four modules, one per reason to change: [`row_sequence`] builds the row
//! sequence, [`cursor`] moves the selection through it, [`queries`] counts
//! what the name filter leaves, and [`rollups`] sums the members a group or
//! fold header stands over.

use super::*;

mod cursor;
mod queries;
mod rollups;
mod row_sequence;
