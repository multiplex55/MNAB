# Financial sign conventions

MNAB uses one sign convention in every domain calculation and persisted amount:

* Inflows are positive; outflows are negative.
* Spending activity is negative; refunds are positive.
* Budget assignments are positive; unassignments are negative.
* Liability balances are normally negative.
* The two sides of a transfer are exact opposites. They must sum to zero using checked arithmetic.

`Money` is integer cents, and calculations must use its checked operations. Imports with more than two decimal places must explicitly select `HalfAwayFromZero`; ordinary parsing rejects such input. USD display uses a leading minus sign (for example, `-$1.00`), not accounting parentheses.
