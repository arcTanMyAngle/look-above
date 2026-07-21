# Static downloads (import scripts, not runtime polling)

- **OurAirports** (public domain): `https://davidmegginson.github.io/ourairports-data/{airports,runways,navaids}.csv`
- **FAA registry** (US): `https://registry.faa.gov/database/ReleasableAircraft.zip` — owner
  names are never displayed (privacy 4.3).
- **openflights airlines** (ODbL): `https://raw.githubusercontent.com/jpatokal/openflights/master/data/airlines.dat`
- **Natural Earth** (public domain): fetched at build/setup time from naturalearthdata.com,
  bundled; never at app runtime.
- All honor ETag/Last-Modified; refresh monthly/quarterly, not per-run.
