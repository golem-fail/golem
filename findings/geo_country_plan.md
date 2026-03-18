# Geo Country Plan: Consolidated Research Review

Ordered list of countries for geo data generation, grouped by wave, with data quality assessments based on Wave 3-4 research findings.

**Assessment criteria:**
- **High**: Excellent/good open postal code data, well-documented admin divisions, clear address format, known phone format. Ready for implementation.
- **Medium**: Adequate data available but with notable gaps, complexity, or licensing concerns. Feasible with extra effort.
- **Low**: Sparse postal code data, incomplete adoption, or significant structural barriers. May need substitution or simplified approach.

---

## Already Completed

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| JP | Japan | High | Japan Post official data (124K entries, free). GeoNames for romanized names. Excellent coverage. |
| GB | United Kingdom | High | Code-Point Open + ONS. 1.7M postcodes, free open data. World-class geographic data. |

---

## Wave 22.3

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| KR | South Korea | High | JUSO system is world-class. GeoNames solid for romanized data. 5-digit codes, free government data. |

---

## Wave 23

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| FR | France | High | La Poste Hexasmal via data.gouv.fr. 6,300 communes, free. INSEE for admin geography. Simple 5-digit codes. |
| IE | Ireland | Medium | Eircode (2015) is commercially restricted. GeoNames may not fully cover Eircodes. Rural addresses unstructured (townland-based). City addresses more feasible. |
| PL | Poland | High | GeoNames ~38.7K entries, excellent coverage. TERYT for admin divisions. Polish characters require handling. ##-### format. |
| BE | Belgium | High | GeoNames ~2,700 entries. StatBel for official data. Simple 4-digit system. Bilingual (NL/FR) naming is main complexity. |

---

## Wave 24

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| LT | Lithuania | Medium | GeoNames provides reasonable coverage. LT- prefix required. Lithuanian characters need handling. Address register access may be restricted. |
| SE | Sweden | High | GeoNames ~17.4K entries, solid coverage. SCB for admin data. Distinctive NNN NN format. Swedish characters (a, a, o). |
| RU | Russia | Medium | FIAS/GAR comprehensive but huge. GeoNames covers major areas. 6-digit codes. Cyrillic script essential. Transliteration varies. Vast geographic size with many rural postcodes. Complex address format. |
| DE | Germany | High | GeoNames ~16.4K entries + OpenPLZ API (open-source, MIT). 5-digit PLZ. Official Deutsche Post data commercially restricted, but community sources adequate. |

---

## Wave 25

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| ES | Spain | High | GeoNames ~37K entries. INE for admin data. Simple 5-digit province-based codes. Regional language names (Catalan, Basque, Galician) are a consideration. |
| IL | Israel | High | GeoNames ~2,500 entries. 7-digit system (since 2013). CBS for locality data. Hebrew RTL handling needed. Diverse naming conventions. |
| US | United States | High | GeoNames ~43K ZIP codes. Census Bureau ZCTA for boundaries. Excellent free data. Simple 5-digit format. |
| CA | Canada | High | GeoNames ~870K entries. Complex A#A #A# format with letter restrictions. FSA-to-province mapping enables validation. Best free source (official data commercially licensed). |

---

## Wave 26

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| CN | China | Medium | China Post has no bulk download. GeoNames ~77K entries but may be incomplete for rural areas. GitHub community datasets good for admin divisions. 6-digit codes less standardized in rural areas. |
| IN | India | High | India Post via data.gov.in: 155K post offices with PIN codes, free official data. Diverse address formats across regions. Multiple scripts add complexity. |
| AU | Australia | High | GeoNames ~16K entries + Matthew Proctor community dataset. Simple 4-digit codes by state. Well-standardized address format. Multiple free sources. |
| SG | Singapore | High | OneMap API is comprehensive. data.gov.sg for bulk datasets. Small geography = complete coverage. 6-digit codes (one per building). Excellent open data ecosystem. |

---

## Wave 27

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| EG | Egypt | High | GeoNames ~3,200 entries, solid coverage. 5-digit system well-established. 27 governorates well-documented. Bilingual Arabic/Latin addressing. |
| ZA | South Africa | High | GeoNames ~3,500 entries. Simple 4-digit codes. 9 provinces well-documented. Suburb is key address component. English-language street names. |
| BR | Brazil | High | GeoNames ~560K entries. ViaCEP/Brasil API for programmatic lookup. 8-digit CEP format. Bairro (neighborhood) field significant. Large dataset but well-sourced. |
| MX | Mexico | High | SEPOMEX official data free (~145K entries). datos.gob.mx for machine-readable formats. 5-digit codes. Colonia (neighborhood) is essential component. |

---

## Wave 28 (Fill Slots)

Recommended countries for Wave 28 based on research data quality:

| ISO | Country | Quality | Notes | Recommendation |
|-----|---------|---------|-------|----------------|
| TH | Thailand | Medium | GeoNames ~2,600 entries. No bulk official download. Thai script localization challenge. RTGS romanization inconsistent. | Include -- feasible with romanized data |
| AE | UAE | Medium | No traditional postal codes. Uses Makani + P.O. Box system. Emirate/city/area/district addressing. | Include -- unique but implementable (7 emirates, descriptive addressing) |
| NZ | New Zealand | High | GeoNames ~1,800 entries. LINZ Data Service excellent (CC BY 4.0). Simple 4-digit codes. Well-standardized format. | **Strongly recommend** -- high quality, straightforward |
| NG | Nigeria | Low | GeoNames only ~850 entries. 6-digit codes not universally adopted. Postal codes rarely used in practice. | Defer or simplify -- use state+city+area instead of postal codes |

### Wave 28 Recommendation

**Recommended lineup:** NZ, TH, AE + one substitute

**Substitution candidates** (if NG is deferred):

| ISO | Country | Quality | Notes |
|-----|---------|---------|-------|
| NL | Netherlands | High | BAG is one of the best open address databases in the world. 4-digit+2-letter format. Excellent data. |
| CH | Switzerland | High | Swiss Post open data: official, free, multilingual. 4-digit PLZ. |
| NO | Norway | High | Posten/Bring provides free official postal code table. 4-digit system. Rare free official data. |
| DK | Denmark | High | DAWA API is world-class. Free, official, complete. 4-digit system. |
| FI | Finland | High | Posti provides free bulk downloads. Bilingual FI/SE. 5-digit system. |
| TW | Taiwan | High | Chunghwa Post provides free official data. Well-organized 3+2 digit system. |

**Top substitute for NG:** NL (Netherlands) -- world-class open data via BAG/Kadaster, unique postal code format, and strong European market relevance.

---

## Countries Reviewed but Not in Current Waves

These countries were researched but are not currently slated for any wave. Available as future additions:

| ISO | Country | Quality | Region | Notes |
|-----|---------|---------|--------|-------|
| IT | Italy | High | Europe | GeoNames ~18.5K. ISTAT excellent. 5-digit CAP. |
| NL | Netherlands | High | Europe | BAG world-class. Unique ####AA format. |
| CH | Switzerland | High | Europe | Swiss Post free open data. Multilingual. |
| AT | Austria | High | Europe | data.gv.at + OpenPLZ API. 4-digit PLZ. |
| NO | Norway | High | Europe | Free official postal code table from Posten/Bring. |
| DK | Denmark | High | Europe | DAWA API world-class. |
| FI | Finland | High | Europe | Free official data from Posti. |
| CZ | Czech Republic | High | Europe | RUIAN open address register. |
| PT | Portugal | High | Europe | GeoNames ~34.5K. 7-digit ####-### format. |
| GR | Greece | Medium | Europe | Dual-script (Greek/Latin). Transliteration inconsistent. |
| TW | Taiwan | High | Asia | Chunghwa Post free official data. |
| ID | Indonesia | Medium | Asia | GeoNames ~7.3K. Vast archipelago. RT/RW addressing unique. |
| VN | Vietnam | Medium | Asia | Postal code system reformed 2017, data stabilizing. |
| PH | Philippines | Medium | Asia | GeoNames reasonable. Barangay layer adds complexity. |
| MY | Malaysia | High | Asia | GeoNames solid. Multi-ethnic naming. |
| AR | Argentina | Medium | Americas | Dual postal code system (4-digit legacy vs 8-char CPA). |
| CO | Colombia | High | Americas | 6-digit codes (2010). Coordinate-based street grid. DANE authoritative. |
| CL | Chile | Medium | Americas | 7-digit codes sparse documentation. GeoNames ~2.2K. |
| PE | Peru | Low | Americas | Least developed postal codes. GeoNames only ~900 entries. UBIGEO more common. |
| KE | Kenya | Medium | Africa | P.O. Box addressing traditional. Street addresses growing in cities. |
| MA | Morocco | High | Africa/ME | GeoNames ~1,500. Bilingual FR/AR. Well-organized 5-digit system. |
| SA | Saudi Arabia | High | Africa/ME | National Address system well-designed. GeoNames ~1,200. |

---

## Summary: Quality Distribution Across Waves

| Wave | Countries | High Quality | Medium Quality | Low Quality |
|------|-----------|-------------|----------------|-------------|
| Done | JP, GB | 2 | 0 | 0 |
| 22.3 | KR | 1 | 0 | 0 |
| 23 | FR, IE, PL, BE | 3 | 1 (IE) | 0 |
| 24 | LT, SE, RU, DE | 2 | 2 (LT, RU) | 0 |
| 25 | ES, IL, US, CA | 4 | 0 | 0 |
| 26 | CN, IN, AU, SG | 3 | 1 (CN) | 0 |
| 27 | EG, ZA, BR, MX | 4 | 0 | 0 |
| 28 | NZ, TH, AE, (NG or sub) | 1 | 2 (TH, AE) | 1 (NG) |
| **Total** | **26** | **20** | **6** | **1** |

### Key Risks

1. **IE (Ireland)** -- Eircode commercially restricted; rural addresses problematic. Mitigation: focus on urban addresses, use GeoNames + available open data.
2. **RU (Russia)** -- Cyrillic handling, vast size, complex address format. Mitigation: focus on major cities (Moscow, St. Petersburg, etc.), use GeoNames for postal codes.
3. **CN (China)** -- No official bulk download, rural gaps. Mitigation: GeoNames + GitHub community datasets for admin divisions.
4. **NG (Nigeria)** -- Postal codes rarely used in practice. Recommendation: substitute with NL (Netherlands) or defer to later wave with simplified state+city approach.
5. **LT (Lithuania)** -- Address register access may be restricted. Mitigation: GeoNames + available open data adequate for fake data generation.
