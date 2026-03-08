# Geo Data Research: Europe

Research findings for generating fake geographic data for European countries.
Covers postal codes, administrative divisions, address formats, phone formats, and name ordering.

---

## Table of Contents

1. [United Kingdom (GB)](#united-kingdom-gb)
2. [France (FR)](#france-fr)
3. [Germany (DE)](#germany-de)
4. [Spain (ES)](#spain-es)
5. [Italy (IT)](#italy-it)
6. [Netherlands (NL)](#netherlands-nl)
7. [Belgium (BE)](#belgium-be)
8. [Switzerland (CH)](#switzerland-ch)
9. [Austria (AT)](#austria-at)
10. [Sweden (SE)](#sweden-se)
11. [Norway (NO)](#norway-no)
12. [Denmark (DK)](#denmark-dk)
13. [Finland (FI)](#finland-fi)
14. [Poland (PL)](#poland-pl)
15. [Czech Republic (CZ)](#czech-republic-cz)
16. [Portugal (PT)](#portugal-pt)
17. [Greece (GR)](#greece-gr)
18. [Ireland (IE)](#ireland-ie)
19. [Lithuania (LT)](#lithuania-lt)
20. [Russia (RU)](#russia-ru)

---

## United Kingdom (GB)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **ONS Postcode Directory (ONSPD)** | https://geoportal.statistics.gov.uk/datasets/ons-postcode-directory | CSV | Official. ~1.7 million postcodes. Includes grid references, local authority, health authority, and constituency mappings. Updated quarterly. Open Government Licence. |
| **Code-Point Open (Ordnance Survey)** | https://osdatahub.os.uk/downloads/open/CodePointOpen | CSV/GeoPackage | Free open data. ~1.7 million postcodes with easting/northing coordinates. Open Government Licence. |
| Royal Mail PAF (Postcode Address File) | https://www.royalmail.com/business/services/marketing/data-optimisation/paf | Proprietary | The most comprehensive (~30 million delivery points) but commercially licensed. Not suitable for open use. |
| GeoNames | https://download.geonames.org/export/zip/GB.zip | TSV | ~27,000 outward code entries. CC BY 4.0. Good for basic postcode-to-place mapping. |
| Doogal | https://www.doogal.co.uk/PostcodeDownloads | CSV | Community-maintained. Derived from open data. Includes lat/long. |

**Recommended primary source:** Code-Point Open for coordinates + GeoNames for place name mapping. ONSPD for comprehensive admin geography linkage.

### Administrative Divisions (Cities/States)

- **4 countries:** England, Scotland, Wales, Northern Ireland
- **England:** 9 regions, 48 ceremonial counties, 309 local authority districts
- **Scotland:** 32 council areas
- **Wales:** 22 principal areas
- **Northern Ireland:** 11 local government districts
- Source: ONS geography lookups at https://geoportal.statistics.gov.uk/
- Wikipedia: https://en.wikipedia.org/wiki/Counties_of_the_United_Kingdom

### Address Format

```
[Building number/name] [Street name]
[Locality (optional)]
[Town/City]
[County (optional)]
[Postcode]

Example:
10 Downing Street
London
SW1A 2AA
```

- **Postal code format:** `A#[#] #AA` or `AA#[#] #AA` (outward code + space + inward code)
  - Valid patterns: `A9 9AA`, `A99 9AA`, `A9A 9AA`, `AA9 9AA`, `AA99 9AA`, `AA9A 9AA`
  - Regex: `^[A-Z]{1,2}[0-9][0-9A-Z]?\s?[0-9][A-Z]{2}$`
- **Street number:** Before street name. Typically 1-999. Can include suffixes (10A, 10B).
- **Street types:** Road, Street, Avenue, Lane, Drive, Close, Way, Place, Crescent, Terrace, Gardens, Grove, Court, Mews
- **Street naming:** English names. Named after people, places, trees, features.

### Phone Number Format

- **Country code:** +44
- **Format patterns:**
  - Landline: `+44 20 #### ####` (London), `+44 1### ######` (other areas), `+44 1#1 ### ####` (major cities)
  - Mobile: `+44 7### ######` (prefixes: 071x-075x, 077x-079x)
  - Total digits (excluding country code): 10
- **Domestic format:** `0## #### ####` or `01### ######`

### Name Ordering

- **given_first** (e.g., James Smith)

### Data Quality Assessment

- **Excellent.** The UK has world-class open geographic data through Ordnance Survey Open Data and ONS. Code-Point Open provides free, authoritative postcode coordinates. The postcode system is well-structured and regex-validatable. Main complexity is the varied postcode formats (6 patterns).

---

## France (FR)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **La Poste Open Data** | https://datanova.laposte.fr/datasets/laposte-hexasmal | CSV/JSON | Official. ~6,300 communes with postal codes. Open Licence. Includes INSEE codes, GPS coordinates, and commune names. Updated regularly. |
| **data.gouv.fr** | https://www.data.gouv.fr/fr/datasets/base-officielle-des-codes-postaux/ | CSV | Official government open data portal mirror. Same La Poste dataset. |
| GeoNames | https://download.geonames.org/export/zip/FR.zip | TSV | ~51,000 entries. Includes DOM-TOM (overseas territories). CC BY 4.0. |
| OpenDataSoft | https://public.opendatasoft.com/explore/dataset/correspondance-code-cedex-code-insee/ | CSV/JSON/API | Additional CEDEX code mappings. |

**Recommended primary source:** La Poste Hexasmal dataset via data.gouv.fr for authoritative commune-to-postcode mapping. GeoNames for broader coverage.

### Administrative Divisions (Cities/States)

- **18 regions** (13 metropolitan + 5 overseas): Auvergne-Rhone-Alpes, Bourgogne-Franche-Comte, Bretagne, Centre-Val de Loire, Corse, Grand Est, Hauts-de-France, Ile-de-France, Normandie, Nouvelle-Aquitaine, Occitanie, Pays de la Loire, Provence-Alpes-Cote d'Azur, Guadeloupe, Guyane, Martinique, La Reunion, Mayotte
- **101 departments** (96 metropolitan + 5 overseas), numbered 01-95, 2A, 2B (Corsica), 971-976
- **~35,000 communes** (municipalities)
- Source: INSEE (Institut national de la statistique) Code Officiel Geographique: https://www.insee.fr/fr/information/2560452
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_France

### Address Format

```
[Recipient name]
[Building number] [Street type] [Street name]
[Postal code] [CITY (uppercase)]

Example:
Jean Dupont
15 Rue de la Paix
75002 PARIS
```

- **Postal code format:** `#####` (5 digits). First two digits = department number (e.g., 75 = Paris, 13 = Bouches-du-Rhone). Range: 01000-98999.
- **Street number:** Before street name. Typically 1-300. Suffixes: bis (B), ter (T), quater (Q) for subdivisions.
- **Street types (voies):** Rue, Avenue, Boulevard, Place, Chemin, Impasse, Allee, Passage, Route, Cours, Quai
- **City name:** Written in UPPERCASE per La Poste convention.

### Phone Number Format

- **Country code:** +33
- **Format patterns:**
  - Landline: `+33 # ## ## ## ##` (area prefixes: 01=Ile-de-France, 02=NW, 03=NE, 04=SE, 05=SW)
  - Mobile: `+33 6 ## ## ## ##` or `+33 7 ## ## ## ##`
  - Total digits (excluding country code): 9
- **Domestic format:** `0# ## ## ## ##` (10 digits with leading 0)

### Name Ordering

- **given_first** (e.g., Jean Dupont)

### Data Quality Assessment

- **Excellent.** France has outstanding open data through data.gouv.fr and La Poste. The postal code system is simple (5 digits, department-based). INSEE provides authoritative administrative geography. The main consideration is handling accented characters in place names (e.g., Beziers, Orleans).

---

## Germany (DE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Deutsche Post Direkt** | https://www.deutschepost.de/de/d/deutsche-post-direkt/datafactory.html | Proprietary | Official source but commercially licensed. Not freely available. |
| **OpenPLZ API** | https://openplzapi.org/ | REST API / JSON | Open-source project. Covers DE, AT, CH, LI. Community-maintained. MIT License. |
| **Suche-Postleitzahl.org** | https://www.suche-postleitzahl.org/downloads | CSV | Community-curated. ~8,200 PLZ entries with lat/long, municipality, state. Free for non-commercial use. |
| GeoNames | https://download.geonames.org/export/zip/DE.zip | TSV | ~16,400 entries. CC BY 4.0. Good coverage. |
| Destatis | https://www.destatis.de/DE/Themen/Laender-Regionen/Regionales/ | Various | Federal Statistical Office. Official municipality registry (Gemeindeverzeichnis). |

**Recommended primary source:** GeoNames for basic PLZ-to-place mapping. OpenPLZ API for a structured, open-source option. Suche-Postleitzahl.org for enriched data.

### Administrative Divisions (Cities/States)

- **16 Bundeslander (federal states):**
  - Baden-Wurttemberg (BW), Bayern/Bavaria (BY), Berlin (BE), Brandenburg (BB), Bremen (HB), Hamburg (HH), Hessen (HE), Mecklenburg-Vorpommern (MV), Niedersachsen (NI), Nordrhein-Westfalen (NW), Rheinland-Pfalz (RP), Saarland (SL), Sachsen (SN), Sachsen-Anhalt (ST), Schleswig-Holstein (SH), Thuringen (TH)
- **401 Kreise** (districts/counties) below states
- **~11,000 Gemeinden** (municipalities)
- Source: Destatis Gemeindeverzeichnis: https://www.destatis.de/DE/Themen/Laender-Regionen/Regionales/Gemeindeverzeichnis/_inhalt.html
- Wikipedia: https://en.wikipedia.org/wiki/States_of_Germany

### Address Format

```
[Recipient name]
[Street name] [Building number]
[Postal code] [City]

Example:
Max Mustermann
Musterstrasse 42
10115 Berlin
```

- **Postal code format:** `#####` (5 digits). Range: 01001-99998. Leading zeros significant (e.g., 01067 = Dresden).
- **Street number:** After street name (unlike UK/US). Typically 1-300. Suffixes: a, b, c for subdivisions.
- **Street types:** -strasse/-str. (Strasse), -weg (Way), -platz (Square), -gasse (Lane), -allee (Avenue), -ring (Ring road), -damm (Embankment)
- **Street naming:** Often compound words (e.g., Friedrichstrasse, Konigstrasse)

### Phone Number Format

- **Country code:** +49
- **Format patterns:**
  - Landline: `+49 ## ########` (variable length area codes: 2-5 digits; subscriber: 3-8 digits)
  - Berlin: `+49 30 ########`, Munich: `+49 89 ########`, Hamburg: `+49 40 ########`
  - Mobile: `+49 1## ########` (prefixes: 015x, 016x, 017x)
  - Total digits (excluding country code): 10-11
- **Domestic format:** `0## ########`

### Name Ordering

- **given_first** (e.g., Max Mustermann)

### Data Quality Assessment

- **Good.** Official Deutsche Post data is commercially restricted, but GeoNames and community sources provide adequate coverage. The PLZ system is straightforward (5 digits). German place names use special characters (umlauts: a, o, u, ss) that must be handled. OpenPLZ API is an excellent open-source alternative.

---

## Spain (ES)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Correos (Spanish Post)** | https://www.correos.es/es/es/herramientas/codigos-postales | Web lookup | Official source but no bulk download freely available. |
| **INE (Instituto Nacional de Estadistica)** | https://www.ine.es/ss/Satellite?L=es_ES&c=Page&cid=1254734710990&p=1254734710990&pagename=ProductosYServicios%2FPYSLayout | Various | Official municipality and province data. |
| GeoNames | https://download.geonames.org/export/zip/ES.zip | TSV | ~37,000 entries. Good coverage. CC BY 4.0. |
| Datos.gob.es | https://datos.gob.es/ | Various | Spanish open data portal. Administrative boundary data available. |

**Recommended primary source:** GeoNames for postal code-to-place mapping. INE for authoritative administrative data.

### Administrative Divisions (Cities/States)

- **17 Comunidades Autonomas (autonomous communities)** + 2 autonomous cities (Ceuta, Melilla):
  - Andalucia, Aragon, Asturias, Islas Baleares, Canarias, Cantabria, Castilla y Leon, Castilla-La Mancha, Cataluna/Catalunya, Comunidad Valenciana, Extremadura, Galicia, Comunidad de Madrid, Region de Murcia, Navarra, Pais Vasco/Euskadi, La Rioja
- **50 provinces** (provincias), each with a two-digit code (01-52)
- **~8,100 municipalities** (municipios)
- Source: INE municipal register: https://www.ine.es/dyngs/INEbase/es/categoria.htm?c=Estadistica_P&cid=1254734710990
- Wikipedia: https://en.wikipedia.org/wiki/Autonomous_communities_of_Spain

### Address Format

```
[Recipient name]
[Street type] [Street name], [Number], [Floor] [Door]
[Postal code] [City] ([Province])

Example:
Maria Garcia Lopez
Calle Mayor, 25, 3o B
28013 Madrid
```

- **Postal code format:** `#####` (5 digits). First two digits = province code (e.g., 28 = Madrid, 08 = Barcelona). Range: 01001-52080.
- **Street number:** After street name, separated by comma. Typically 1-200.
- **Floor/door:** Common in apartments: `1o A`, `3o B`, `Bajo` (ground floor), `Atico` (penthouse).
- **Street types:** Calle (C/), Avenida (Avda.), Plaza (Pl.), Paseo (P.), Carretera (Ctra.), Camino, Ronda, Travesia

### Phone Number Format

- **Country code:** +34
- **Format patterns:**
  - Landline: `+34 9## ### ###` (geographic numbers start with 9, area embedded in number)
  - Mobile: `+34 6## ### ###` or `+34 7## ### ###`
  - Total digits: 9 (no separate area code; all 9 digits dialed)
- **Domestic format:** `### ### ###` (no leading 0 needed)

### Name Ordering

- **given_first** (e.g., Maria Garcia Lopez)
- Note: Two surnames common (paternal + maternal). For fake data, include both.

### Data Quality Assessment

- **Good.** GeoNames provides solid postal code coverage. The Spanish postal code system is simple (5 digits, province-based). Main considerations: handling regional language names (Catalan, Basque, Galician) and the dual-surname convention. No freely downloadable bulk postal code dataset from Correos.

---

## Italy (IT)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Poste Italiane** | https://www.poste.it/cerca/index.html#/vieni-a-trovarci | Web lookup | Official source. No free bulk download. |
| **ISTAT** | https://www.istat.it/it/archivio/6789 | CSV/Excel | Official statistical institute. Municipality codes, province codes, region codes. Not postal codes directly. |
| GeoNames | https://download.geonames.org/export/zip/IT.zip | TSV | ~18,500 entries. CC BY 4.0. Good coverage. |
| CAP database (community) | Various GitHub repos | CSV | Community-maintained CAP (Codice di Avviamento Postale) lists. |

**Recommended primary source:** GeoNames for postal code-to-place mapping. ISTAT for administrative geography.

### Administrative Divisions (Cities/States)

- **20 regions** (regioni), 5 with special statute:
  - Abruzzo, Basilicata, Calabria, Campania, Emilia-Romagna, Friuli Venezia Giulia*, Lazio, Liguria, Lombardia, Marche, Molise, Piemonte, Puglia, Sardegna*, Sicilia*, Toscana, Trentino-Alto Adige/Sudtirol*, Umbria, Valle d'Aosta/Vallee d'Aoste*, Veneto
- **107 provinces/metropolitan cities** (province/citta metropolitane), each with a two-letter code (e.g., RM=Roma, MI=Milano, NA=Napoli)
- **~7,900 comuni** (municipalities)
- Source: ISTAT administrative codes: https://www.istat.it/it/archivio/6789
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Italy

### Address Format

```
[Recipient name]
[Street type] [Street name], [Number]
[Postal code] [CITY] [Province code]

Example:
Marco Rossi
Via Roma, 15
00187 ROMA RM
```

- **Postal code format (CAP):** `#####` (5 digits). Range: 00010-98168. First digit indicates macro-area (0=NW/Central, 1-2=NE, 3=Veneto/Emilia, 4-5=Central-South, 6-7=South, 8=Calabria/Basilicata, 9=Sicily/Sardinia).
- **Street number:** After street name, separated by comma. Typically 1-300. Can include letter suffixes (15/A, 15/B).
- **Street types:** Via, Viale, Piazza (Pza./P.zza), Corso, Largo, Vicolo, Piazzale, Lungarno, Borgo
- **City:** Written in UPPERCASE. Province abbreviation (2-letter) appended.

### Phone Number Format

- **Country code:** +39
- **Format patterns:**
  - Landline: `+39 0# #### ####` (area codes start with 0, included in international dialing)
  - Rome: `+39 06 ########`, Milan: `+39 02 ########`
  - Mobile: `+39 3## ### ####` (prefixes: 320-339, 340-349, 360-368, 380-389, 390-399)
  - Total digits: 9-11 (variable length)
- **Domestic format:** Same as international (no leading 0 dropped; the 0 is part of the area code)
- **Note:** Italy is unusual in that the leading 0 of area codes must be dialed even for international calls.

### Name Ordering

- **given_first** (e.g., Marco Rossi)

### Data Quality Assessment

- **Good.** GeoNames provides adequate postal code data. ISTAT is an excellent source for administrative geography. The CAP system is straightforward (5 digits). Italian place names are relatively simple to handle. The province abbreviation convention (2-letter code after city) is important for realistic addresses.

---

## Netherlands (NL)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **PostNL** | https://www.postnl.nl/ | Proprietary | Official postal service. Bulk data commercially licensed. |
| **CBS (Centraal Bureau voor de Statistiek)** | https://www.cbs.nl/nl-nl/dossier/nederland-regionaal/geografische-data/gegevens-per-postcode | CSV | Official statistics. PC4 (4-digit) level data. Free. Includes population, area statistics. |
| GeoNames | https://download.geonames.org/export/zip/NL.zip | TSV | ~4,600 PC4-level entries. CC BY 4.0. |
| PDOK (Publieke Dienstverlening Op de Kaart) | https://www.pdok.nl/ | Various (WFS/WMS/API) | Government geo-services. Very detailed. Open data. |
| Kadaster (BAG) | https://www.kadaster.nl/zakelijk/producten/adressen-en-gebouwen/bag-2.0-extract | XML/CSV | Basisregistratie Adressen en Gebouwen. Most comprehensive Dutch address database. Open data. ~9.5 million addresses. |

**Recommended primary source:** BAG (Kadaster) for comprehensive address data. CBS PC4 for statistical postcode areas. GeoNames for simpler postcode-place mapping.

### Administrative Divisions (Cities/States)

- **12 provinces** (provincies): Drenthe, Flevoland, Friesland/Fryslan, Gelderland, Groningen, Limburg, Noord-Brabant, Noord-Holland, Overijssel, Utrecht, Zeeland, Zuid-Holland
- **342 gemeenten** (municipalities, as of 2024 -- decreasing due to mergers)
- Source: CBS regional classifications: https://www.cbs.nl/nl-nl/onze-diensten/methoden/classificaties
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_the_Netherlands

### Address Format

```
[Recipient name]
[Street name] [Number][Suffix]
[Postal code]  [City]

Example:
Jan de Vries
Keizersgracht 123-II
1015 CJ  AMSTERDAM
```

- **Postal code format:** `#### AA` (4 digits + space + 2 uppercase letters). Range: 1000 AA - 9999 ZZ. Letters SA, SD, SS excluded (resemblance to Nazi terms).
- **Street number:** After street name. Typically 1-999. Suffixes common: -I, -II, -III (floor), -A, -B (apartment), -hs (huis/ground floor), -bg (begane grond).
- **Street types:** Embedded in name — -straat, -weg, -laan, -gracht, -kade, -plein, -singel, -dijk
- **City:** Written in UPPERCASE per PostNL convention. Double space between postcode and city.

### Phone Number Format

- **Country code:** +31
- **Format patterns:**
  - Landline: `+31 ## ### ####` (2-digit area codes for major cities: 20=Amsterdam, 70=The Hague, 10=Rotterdam) or `+31 ### ### ###` (3-digit area codes)
  - Mobile: `+31 6 ########` (all mobile numbers start with 06)
  - Total digits (excluding country code): 9
- **Domestic format:** `0## ### ####` or `06 ########`

### Name Ordering

- **given_first** (e.g., Jan de Vries)
- Note: Tussenvoegsels (particles like "van", "de", "van der", "van den") are common and not capitalized when preceded by given name.

### Data Quality Assessment

- **Excellent.** The Netherlands has outstanding open geographic data. The BAG (national address register) is one of the most complete open address databases in the world. The postal code system (4 digits + 2 letters) is unique and well-structured. CBS provides excellent statistical data at PC4 level.

---

## Belgium (BE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **bpost** | https://www.bpost.be/nl/postcodes | Web lookup | Official postal service. No free bulk download. |
| **StatBel (Belgian Statistical Office)** | https://statbel.fgov.be/en/open-data | Various | Open data portal. Municipality data with postal codes. |
| GeoNames | https://download.geonames.org/export/zip/BE.zip | TSV | ~2,700 entries. CC BY 4.0. |
| data.gov.be | https://data.gov.be/ | Various | Belgian open data portal. Administrative boundary data. |

**Recommended primary source:** GeoNames for postal code-place mapping. StatBel for official administrative data.

### Administrative Divisions (Cities/States)

- **3 regions:** Brussels-Capital (Bruxelles-Capitale/Brussels Hoofdstedelijk), Flanders (Vlaanderen), Wallonia (Wallonie)
- **10 provinces** (5 Flemish, 5 Walloon) + Brussels-Capital Region:
  - Flemish: Antwerpen, Limburg, Oost-Vlaanderen, Vlaams-Brabant, West-Vlaanderen
  - Walloon: Brabant wallon, Hainaut, Liege, Luxembourg, Namur
- **581 gemeenten/communes** (municipalities)
- **3 language communities:** Dutch, French, German
- Source: StatBel: https://statbel.fgov.be/
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_Belgium

### Address Format

```
[Recipient name]
[Street name] [Number]
[Postal code] [City]

Example (Dutch):
Pieter Janssens
Grote Markt 1
1000 Brussel

Example (French):
Pierre Janssens
Grand-Place 1
1000 Bruxelles
```

- **Postal code format:** `####` (4 digits). Range: 1000-9999. First digit indicates province region (1=Brussels/Brabant, 2=Antwerpen, 3=Limburg/Vlaams-Brabant, 4=Liege/Luxembourg, 5=Namur/Hainaut, 6=Luxembourg/Hainaut, 7=Hainaut, 8=West-Vlaanderen, 9=Oost-Vlaanderen).
- **Street number:** After street name. Typically 1-300.
- **Street types (NL/FR):** Straat/Rue, Laan/Avenue, Plein/Place, Steenweg/Chaussee, Weg/Chemin
- **Bilingual:** Brussels addresses often in both Dutch and French. Flanders uses Dutch names; Wallonia uses French names.

### Phone Number Format

- **Country code:** +32
- **Format patterns:**
  - Landline: `+32 # ### ## ##` (1-digit area: 2=Brussels, 3=Antwerp, 4=Liege, 9=Ghent) or `+32 ## ## ## ##` (2-digit areas)
  - Mobile: `+32 4## ## ## ##` (prefixes: 046x-049x)
  - Total digits (excluding country code): 8-9
- **Domestic format:** `0# ### ## ##` or `04## ## ## ##`

### Name Ordering

- **given_first** (e.g., Pieter Janssens)

### Data Quality Assessment

- **Good.** Belgium's bilingual/trilingual nature adds complexity. GeoNames covers postal codes well. The 4-digit system is simple. Main challenge: municipality names exist in Dutch, French, and sometimes German variants (e.g., Liege/Luik, Bruxelles/Brussel, Mons/Bergen). Must decide which language to use based on region.

---

## Switzerland (CH)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Swiss Post (Die Post/La Poste)** | https://swisspost.opendatasoft.com/explore/dataset/plz_verzeichnis_v2/ | CSV/JSON/API | Official open data. ~4,200 entries. Includes all four language variants. Updated regularly. Excellent quality. |
| **OpenPLZ API** | https://openplzapi.org/ | REST API / JSON | Open-source. Covers CH alongside DE, AT. MIT License. |
| GeoNames | https://download.geonames.org/export/zip/CH.zip | TSV | ~4,200 entries. CC BY 4.0. |
| Federal Statistical Office (BFS) | https://www.bfs.admin.ch/bfs/en/home/basics/nomenclatures.html | Various | Official municipality list with FSO codes. |

**Recommended primary source:** Swiss Post open data portal (best quality, official, free, multilingual).

### Administrative Divisions (Cities/States)

- **26 cantons** (Kantone/cantons/cantoni):
  - Zurich (ZH), Bern (BE), Luzern (LU), Uri (UR), Schwyz (SZ), Obwalden (OW), Nidwalden (NW), Glarus (GL), Zug (ZG), Fribourg/Freiburg (FR), Solothurn (SO), Basel-Stadt (BS), Basel-Landschaft (BL), Schaffhausen (SH), Appenzell Ausserrhoden (AR), Appenzell Innerrhoden (AI), St. Gallen (SG), Graubunden/Grischun/Grigioni (GR), Aargau (AG), Thurgau (TG), Ticino (TI), Vaud (VD), Valais/Wallis (VS), Neuchatel (NE), Geneve (GE), Jura (JU)
- **~2,100 Gemeinden/communes** (decreasing due to mergers)
- **4 official languages:** German (~63%), French (~23%), Italian (~8%), Romansh (~0.5%)
- Source: BFS official commune register: https://www.bfs.admin.ch/bfs/en/home/basics/nomenclatures.html
- Wikipedia: https://en.wikipedia.org/wiki/Cantons_of_Switzerland

### Address Format

```
[Recipient name]
[Street name] [Number]
[Postal code] [City]

Example:
Hans Muller
Bahnhofstrasse 15
8001 Zurich
```

- **Postal code format (NPA/PLZ):** `####` (4 digits). Range: 1000-9658. First digit indicates region (1=Romandie, 2=Romandie, 3=Bern, 4=Basel, 5=NW Switzerland, 6=Central, 7=Graubunden, 8=Zurich/East, 9=St. Gallen/East).
- **Street number:** After street name. Typically 1-200.
- **Street types:** Language-dependent. German: -strasse, -weg, -gasse, -platz; French: Rue, Avenue, Chemin, Place; Italian: Via, Piazza, Viale.

### Phone Number Format

- **Country code:** +41
- **Format patterns:**
  - Landline: `+41 ## ### ## ##` (2-digit area codes: 44=Zurich, 31=Bern, 22=Geneva, 21=Lausanne)
  - Mobile: `+41 7# ### ## ##` (prefixes: 076, 077, 078, 079)
  - Total digits (excluding country code): 9
- **Domestic format:** `0## ### ## ##`

### Name Ordering

- **given_first** (e.g., Hans Muller)

### Data Quality Assessment

- **Excellent.** Swiss Post provides one of the best open postal code datasets available -- official, free, well-structured, and multilingual. The 4-digit PLZ system is simple. Main complexity is multilingual place names (same city can have different names in different languages, e.g., Geneve/Genf, Bern/Berne, Fribourg/Freiburg).

---

## Austria (AT)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Osterreichische Post** | https://www.post.at/en/business-address-data.php | Proprietary | Official but commercially licensed for bulk data. |
| **OpenPLZ API** | https://openplzapi.org/ | REST API / JSON | Open-source. Covers AT alongside DE, CH. MIT License. |
| **data.gv.at** | https://www.data.gv.at/katalog/dataset/postleitzahlenverzeichnis | CSV | Austrian open government data. Official PLZ list. Free. |
| GeoNames | https://download.geonames.org/export/zip/AT.zip | TSV | ~2,500 entries. CC BY 4.0. |
| Statistik Austria | https://www.statistik.at/web_de/klassifikationen/regionale_gliederungen/ | Various | Official statistical classifications. Municipality register. |

**Recommended primary source:** data.gv.at for official open data. OpenPLZ API for structured access. GeoNames for broad coverage.

### Administrative Divisions (Cities/States)

- **9 Bundeslander (federal states):**
  - Burgenland (B), Karnten/Carinthia (K), Niederosterreich/Lower Austria (NO), Oberosterreich/Upper Austria (OO), Salzburg (S), Steiermark/Styria (ST), Tirol/Tyrol (T), Vorarlberg (V), Wien/Vienna (W)
- **79 Bezirke** (political districts) + 15 statutory cities
- **~2,100 Gemeinden** (municipalities)
- Source: Statistik Austria Gemeindeverzeichnis
- Wikipedia: https://en.wikipedia.org/wiki/States_of_Austria

### Address Format

```
[Recipient name]
[Street name] [Number][/Staircase/Door]
[Postal code] [City]

Example:
Maria Gruber
Hauptstrasse 23/2/5
1010 Wien
```

- **Postal code format (PLZ):** `####` (4 digits). Range: 1010-9992. First digit indicates region (1=Wien, 2=NE, 3=NW, 4=Upper Austria, 5=Salzburg/West, 6=Tyrol, 7=Burgenland, 8=Styria, 9=Carinthia/South).
- **Street number:** After street name. Apartment notation: `[Number]/[Staircase]/[Door]` (e.g., 23/2/5 = building 23, staircase 2, door 5).
- **Street types:** -strasse, -gasse, -weg, -platz, -ring (same as German conventions)

### Phone Number Format

- **Country code:** +43
- **Format patterns:**
  - Landline: `+43 # ########` (Wien: 1) or `+43 ### #####` (other areas, variable length)
  - Mobile: `+43 6## #######` (prefixes: 0650, 0660, 0664, 0676, 0680, 0681, 0699)
  - Total digits (excluding country code): 10-13 (highly variable)
- **Domestic format:** `0# ########`

### Name Ordering

- **given_first** (e.g., Maria Gruber)

### Data Quality Assessment

- **Good.** Austria has decent open data through data.gv.at. The PLZ system is simple (4 digits). German language conventions apply. OpenPLZ API provides a convenient unified API for AT/DE/CH data. Very similar address conventions to Germany.

---

## Sweden (SE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **PostNord (Sweden)** | https://www.postnord.se/vara-verktyg/soka-postnummer | Web lookup | Official postal service. Bulk data commercially licensed. |
| **SCB (Statistiska centralbyran)** | https://www.scb.se/hitta-statistik/ | Various | Swedish statistics office. Geographic data available. |
| GeoNames | https://download.geonames.org/export/zip/SE.zip | TSV | ~17,400 entries. CC BY 4.0. Good coverage. |
| Oppna Data (Swedish open data) | https://www.dataportal.se/ | Various | Swedish national open data portal. |

**Recommended primary source:** GeoNames for postal code data. SCB for administrative geography.

### Administrative Divisions (Cities/States)

- **21 lan (counties):**
  - Blekinge, Dalarna, Gavleborg, Gotland, Halland, Jamtland, Jonkoping, Kalmar, Kronoberg, Norrbotten, Orebro, Ostergotland, Skane, Sodermanland, Stockholm, Uppsala, Varmland, Vasterbotten, Vasternorrland, Vastmanland, Vastra Gotaland
- **290 kommuner** (municipalities)
- Source: SCB regional divisions: https://www.scb.se/
- Wikipedia: https://en.wikipedia.org/wiki/Counties_of_Sweden

### Address Format

```
[Recipient name]
[Street name] [Number]
[Postal code] [CITY]

Example:
Erik Johansson
Storgatan 12
114 51 STOCKHOLM
```

- **Postal code format:** `### ##` (3 digits + space + 2 digits). Range: 100 00 - 984 99. First digit indicates region.
- **Street number:** After street name. Typically 1-200. Apartment: `lgh ####` or `[Number], # tr` (floor).
- **Street types:** -gatan (street), -vagen (road), -stigen (path), -torget (square), -platsen (place), -grand (lane), -backen (hill)
- **City:** Written in UPPERCASE per PostNord convention.

### Phone Number Format

- **Country code:** +46
- **Format patterns:**
  - Landline: `+46 # ### ## ##` (Stockholm: 08) or `+46 ## ### ## ##` (other areas)
  - Mobile: `+46 7# ### ## ##` (prefixes: 070, 072, 073, 076, 079)
  - Total digits (excluding country code): 7-9 (variable)
- **Domestic format:** `0#-### ## ##` or `0##-### ## ##` (hyphen after area code)

### Name Ordering

- **given_first** (e.g., Erik Johansson)

### Data Quality Assessment

- **Good.** GeoNames provides solid postal code coverage. The Swedish postal code format (NNN NN with space) is distinctive. PostNord data is commercially restricted for bulk use. Swedish characters (a, a, o) must be handled. SCB provides good administrative data.

---

## Norway (NO)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Posten (Bring)** | https://www.bring.no/tjenester/adressetjenester/postnummer/postnummertabellen-nedlasting | TSV | Official postal code register. Free download! ~5,000 entries. Tab-separated. Includes municipality codes. Updated weekly. |
| GeoNames | https://download.geonames.org/export/zip/NO.zip | TSV | ~4,700 entries. CC BY 4.0. |
| Kartverket (Norwegian Mapping Authority) | https://www.kartverket.no/en/data/open-data | Various | Official geographic data. Matrikkelen (cadastre). |
| data.norge.no | https://data.norge.no/ | Various | Norwegian open data portal. |

**Recommended primary source:** Posten/Bring official postal code table (free, official, regularly updated -- rare for a national post service).

### Administrative Divisions (Cities/States)

- **11 fylker (counties)** (reduced from 19 in 2020 reform, some re-split in 2024):
  - Agder, Innlandet, More og Romsdal, Nordland, Oslo, Rogaland, Troms, Finnmark, Trondelag, Vestfold og Telemark, Vestland, Viken
  - Note: County structure has been in flux; verify current list.
- **356 kommuner** (municipalities)
- Source: Kartverket / SSB (Statistisk sentralbyra)
- Wikipedia: https://en.wikipedia.org/wiki/Counties_of_Norway

### Address Format

```
[Recipient name]
[Street name] [Number][Letter]
[Postal code] [CITY]

Example:
Lars Hansen
Karl Johans gate 22B
0026 OSLO
```

- **Postal code format:** `####` (4 digits). Range: 0001-9991. First digit indicates rough region (0=Oslo area, 1=Ostlandet East, 2-3=Ostlandet, 4=Sorlandet, 5=Vestlandet, 6=More og Romsdal, 7=Trondelag, 8=Nordland, 9=Troms/Finnmark).
- **Street number:** After street name. Letter suffixes common (22A, 22B).
- **Street types:** gate/gata (street), vei/veien (road), plass/plassen (square), sti/stien (path), torg/torget (market square)
- **City:** UPPERCASE per Posten convention.

### Phone Number Format

- **Country code:** +47
- **Format patterns:**
  - Landline: `+47 ## ## ## ##` (no separate area code; all numbers are 8 digits)
  - Mobile: `+47 4# ## ## ##` or `+47 9# ## ## ##`
  - Total digits: 8 (flat numbering, no area codes)
- **Domestic format:** `## ## ## ##` (no leading 0)

### Name Ordering

- **given_first** (e.g., Lars Hansen)

### Data Quality Assessment

- **Excellent.** Norway stands out because Posten/Bring provides the official postal code table as a free download -- one of the few national post services to do so. Data is clean, well-structured, and frequently updated. The 4-digit system is simple. Norwegian characters (ae, o, a) need handling.

---

## Denmark (DK)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **PostNord (Denmark)** | https://www.postnord.dk/kundeservice/postnummerkort | Web lookup | Official postal service. Bulk data restricted. |
| **DAWA (Danmarks Adressers Web API)** | https://dawa.aws.dk/ | REST API / JSON/CSV | Official Danish address web API. Exceptional quality. Free. All Danish addresses with coordinates. ~3.5 million address points. |
| GeoNames | https://download.geonames.org/export/zip/DK.zip | TSV | ~1,200 entries. CC BY 4.0. |
| Dataforsyningen | https://dataforsyningen.dk/ | Various | Danish government data distribution. Geographic data. |

**Recommended primary source:** DAWA -- outstanding official API with complete address data, free to use. One of the best address APIs in the world.

### Administrative Divisions (Cities/States)

- **5 regioner (regions):** Hovedstaden (Capital), Midtjylland (Central Jutland), Nordjylland (North Jutland), Sjaelland (Zealand), Syddanmark (Southern Denmark)
- **98 kommuner** (municipalities)
- Source: DAWA API / Danmarks Statistik: https://www.dst.dk/
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Denmark

### Address Format

```
[Recipient name]
[Street name] [Number], [Floor]. [Side]
[Postal code] [City]

Example:
Anders Nielsen
Vestergade 12, 3. tv.
8000 Aarhus C
```

- **Postal code format:** `####` (4 digits). Range: 0800-9990. Copenhagen: 1000-2999, Sjaelland: 3000-4999, Fyn: 5000-5999, Jutland: 6000-9990.
- **Street number:** After street name. Floor/side notation: `[number], [floor]. [side]`. Side: tv. (venstre=left), th. (hojre=right), mf. (midt for=middle).
- **Street types:** -vej (road), -gade (street), -alle (avenue), -plads (square), -stien (path), -vangen, -parken
- **City suffix:** Major cities can have district letter (e.g., Aarhus C, Kobenhavn K, Kobenhavn V).

### Phone Number Format

- **Country code:** +45
- **Format patterns:**
  - Landline: `+45 ## ## ## ##` (no area codes; flat 8-digit numbering)
  - Mobile: `+45 ## ## ## ##` (same format; mobile prefixes: 20-31, 40-42, 50-53, 60-61, 71, 81, 91-93)
  - Total digits: 8
- **Domestic format:** `## ## ## ##` (no leading 0)

### Name Ordering

- **given_first** (e.g., Anders Nielsen)

### Data Quality Assessment

- **Excellent.** Denmark's DAWA API is world-class -- a free, official, comprehensive address API covering every address in Denmark. Postal code system is simple (4 digits). Danish characters (ae, o, a) need handling. DAWA makes Denmark one of the easiest countries for address data.

---

## Finland (FI)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Posti (Finnish Post)** | https://www.posti.fi/en/for-corporate-customers/customer-support/postal-code-services/download-postal-code-files | CSV | Official. Free download available! ~3,100 postal code areas. Finnish and Swedish names. Updated regularly. |
| GeoNames | https://download.geonames.org/export/zip/FI.zip | TSV | ~3,500 entries. CC BY 4.0. |
| Tilastokeskus (Statistics Finland) | https://www.stat.fi/tup/paavo/index_en.html | Various | Paavo -- postal code area statistics. Open data with demographics per postal code. |
| Avoindata.fi | https://www.avoindata.fi/ | Various | Finnish open data portal. |

**Recommended primary source:** Posti official download (free, official, bilingual Finnish/Swedish). Tilastokeskus Paavo for enriched data.

### Administrative Divisions (Cities/States)

- **19 regions (maakunnat):** Ahvenanmaa/Aland, Etela-Karjala, Etela-Pohjanmaa, Etela-Savo, Kainuu, Kanta-Hame, Keski-Pohjanmaa, Keski-Suomi, Kymenlaakso, Lappi, Paijat-Hame, Pirkanmaa, Pohjanmaa, Pohjois-Karjala, Pohjois-Pohjanmaa, Pohjois-Savo, Satakunta, Uusimaa, Varsinais-Suomi
- **309 kuntaa/kommuner** (municipalities)
- **Two official languages:** Finnish and Swedish. Many place names have both variants (e.g., Helsinki/Helsingfors, Turku/Abo).
- Source: Statistics Finland: https://www.stat.fi/
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Finland

### Address Format

```
[Recipient name]
[Street name] [Number] [Staircase] [Apartment]
[Postal code] [CITY]

Example:
Matti Virtanen
Mannerheimintie 15 A 7
00250 HELSINKI
```

- **Postal code format:** `#####` (5 digits). Range: 00100-99999. First two digits indicate region (00-02=Helsinki area, 20-21=Turku, 33=Tampere, 90=Oulu).
- **Street number:** After street name. Staircase letter + apartment number follow (e.g., 15 A 7 = building 15, staircase A, apartment 7).
- **Street types:** -katu/-gatan (street), -tie/-vagen (road), -kuja/-grand (lane), -polku/-stigen (path), -tori/-torget (square)
- **City:** UPPERCASE. Bilingual areas show Finnish name (Swedish equivalent in parentheses on official forms).

### Phone Number Format

- **Country code:** +358
- **Format patterns:**
  - Landline: `+358 # ### ####` (area codes 1-digit: 9=Helsinki or 2-digit: 13-19, 2x, 3x, 5x, 6x, 8x)
  - Mobile: `+358 4# ### ####` or `+358 50 ### ####` (prefixes: 040, 041, 044, 045, 046, 050)
  - Total digits (excluding country code): 6-10 (variable length)
- **Domestic format:** `0# ### ####` or `0## ### ####`

### Name Ordering

- **given_first** (e.g., Matti Virtanen)

### Data Quality Assessment

- **Excellent.** Finland is another country where the national post (Posti) provides free bulk postal code downloads. Bilingual (Finnish/Swedish) naming adds some complexity. Statistics Finland provides excellent enriched data. The postal code system is straightforward (5 digits).

---

## Poland (PL)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Poczta Polska** | https://www.poczta-polska.pl/ | Web lookup | Official postal service. No free bulk download. |
| **GUS (Glowny Urzad Statystyczny)** | https://stat.gov.pl/ | Various | Central Statistical Office. TERYT register of territorial divisions. |
| GeoNames | https://download.geonames.org/export/zip/PL.zip | TSV | ~38,700 entries. CC BY 4.0. Very good coverage. |
| TERYT (National Register of Territorial Divisions) | https://eteryt.stat.gov.pl/ | XML/CSV | Official register. Municipality, locality, and street names. Authoritative source. |
| dane.gov.pl | https://dane.gov.pl/ | Various | Polish open data portal. |

**Recommended primary source:** GeoNames for postal codes. TERYT for authoritative administrative divisions and locality names.

### Administrative Divisions (Cities/States)

- **16 wojewodztw (voivodeships/provinces):**
  - Dolnoslaskie (Lower Silesia), Kujawsko-Pomorskie, Lodzkie, Lubelskie, Lubuskie, Malopolskie (Lesser Poland), Mazowieckie (Masovia), Opolskie, Podkarpackie, Podlaskie, Pomorskie (Pomerania), Slaskie (Silesia), Swietokrzyskie, Warminsko-Mazurskie, Wielkopolskie (Greater Poland), Zachodniopomorskie (West Pomerania)
- **380 powiats** (counties/districts)
- **~2,500 gminas** (municipalities)
- Source: GUS/TERYT: https://eteryt.stat.gov.pl/
- Wikipedia: https://en.wikipedia.org/wiki/Voivodeships_of_Poland

### Address Format

```
[Recipient name]
[Street type] [Street name] [Number][/Apartment]
[Postal code] [City]

Example:
Jan Kowalski
ul. Marszalkowska 15/7
00-626 Warszawa
```

- **Postal code format:** `##-###` (2 digits, hyphen, 3 digits). Range: 00-001 to 99-440. First two digits indicate region/city.
- **Street number:** After street name. Apartment notation: `[building]/[apartment]` (e.g., 15/7).
- **Street types:** ul. (ulica=street), al. (aleja=avenue), pl. (plac=square), os. (osiedle=housing estate), skwer, bulwar
- **Street names:** Commonly named after people, historical events, dates (e.g., ul. 3 Maja).

### Phone Number Format

- **Country code:** +48
- **Format patterns:**
  - Landline: `+48 ## ### ## ##` (geographic area codes embedded: 22=Warsaw, 12=Krakow, 61=Poznan)
  - Mobile: `+48 ### ### ###` (prefixes: 50x, 51x, 53x, 60x, 66x, 69x, 72x, 78x, 79x, 88x)
  - Total digits: 9 (flat numbering after country code)
- **Domestic format:** `### ### ###` (no leading 0 needed since 2003)

### Name Ordering

- **given_first** (e.g., Jan Kowalski)

### Data Quality Assessment

- **Good.** GeoNames provides excellent postal code coverage for Poland (~38K entries). TERYT is an authoritative but complex administrative register. Polish characters (a, c, e, l, n, o, s, z, z) must be handled. The postal code format with hyphen (##-###) is distinctive.

---

## Czech Republic (CZ)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Ceska Posta** | https://www.ceskaposta.cz/ke-stazeni/zasilani-702 | Various | Official postal service. Some data available for download. |
| **RUIAN (Registr Uzemni Identifikace, Adres a Nemovitosti)** | https://vdp.cuzk.cz/ | XML/CSV | Official Czech address register. Comprehensive. Managed by CUZK (Czech Office for Surveying, Mapping and Cadastre). Free download. |
| GeoNames | https://download.geonames.org/export/zip/CZ.zip | TSV | ~16,700 entries. CC BY 4.0. |
| Otevrena data (Czech open data) | https://data.gov.cz/ | Various | Czech open data portal. |

**Recommended primary source:** RUIAN for comprehensive address data. GeoNames for postal code mapping.

### Administrative Divisions (Cities/States)

- **13 kraje (regions)** + Prague (capital city with regional status):
  - Hlavni mesto Praha (Prague), Stredocesky, Jihocesky, Plzensky, Karlovarsky, Ustecky, Liberecky, Kralovehradecky, Pardubicky, Vysocina, Jihomoravsky, Olomoucky, Zlinsky, Moravskoslezsky
- **77 okresy** (districts)
- **~6,250 obci** (municipalities)
- Source: CUZK/RUIAN, Czech Statistical Office: https://www.czso.cz/
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_the_Czech_Republic

### Address Format

```
[Recipient name]
[Street name] [Descriptive number]/[Orientation number]
[Postal code] [City]

Example:
Jan Novak
Vaclavske namesti 846/1
110 00 Praha 1
```

- **Postal code format (PSC):** `### ##` (3 digits + space + 2 digits). Range: 100 00 - 798 62. First digit: 1=Prague, 2=Central Bohemia, 3=West/South Bohemia, 4=North Bohemia, 5=East Bohemia, 6=South Moravia, 7=North Moravia.
- **Street number:** Czech system uses dual numbering: descriptive number (cislo popisne, red plate) and orientation number (cislo orientacni, blue plate), written as `[descriptive]/[orientation]`.
- **Street types:** ulice (street), namesti (square), trida (avenue/boulevard), nabrezi (embankment/quay)

### Phone Number Format

- **Country code:** +420
- **Format patterns:**
  - Landline: `+420 ### ### ###` (geographic: 2xx=Prague, 3xx-5xx=other regions)
  - Mobile: `+420 ### ### ###` (prefixes: 601-608, 702-705, 720-739, 770-779)
  - Total digits: 9 (flat numbering, no area codes since 2002)
- **Domestic format:** `### ### ###` (no leading 0)

### Name Ordering

- **given_first** (e.g., Jan Novak)

### Data Quality Assessment

- **Good.** RUIAN is an excellent official address register (one of the few open national address databases in Europe). GeoNames provides good postal code coverage. The dual house numbering system (descriptive/orientation) is unique and must be understood for realistic addresses. Czech characters (c, d, e, n, r, s, t, u, z) need handling.

---

## Portugal (PT)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **CTT (Correios de Portugal)** | https://www.ctt.pt/feapl_2/app/open/codPostal/codigos_postais.jspx | Web lookup / downloadable | Official postal service. 7-digit postal codes. Some download options available. |
| GeoNames | https://download.geonames.org/export/zip/PT.zip | TSV | ~34,500 entries. CC BY 4.0. Good coverage. |
| dados.gov.pt | https://dados.gov.pt/ | Various | Portuguese open data portal. |
| CAOP (Carta Administrativa Oficial de Portugal) | https://www.dgterritorio.gov.pt/ | Shapefile/GeoJSON | Official administrative boundaries. From DGT (Direcao-Geral do Territorio). |

**Recommended primary source:** GeoNames for postal code mapping. CTT for authoritative postal code data.

### Administrative Divisions (Cities/States)

- **18 districts** (distritos, mainland) + 2 autonomous regions:
  - Mainland: Aveiro, Beja, Braga, Braganca, Castelo Branco, Coimbra, Evora, Faro, Guarda, Leiria, Lisboa, Portalegre, Porto, Santarem, Setubal, Viana do Castelo, Vila Real, Viseu
  - Autonomous regions: Acores (Azores), Madeira
- **308 municipios** (municipalities/concelhos)
- **~3,100 freguesias** (civil parishes)
- Source: INE (Instituto Nacional de Estatistica): https://www.ine.pt/
- Wikipedia: https://en.wikipedia.org/wiki/Districts_of_Portugal

### Address Format

```
[Recipient name]
[Street type] [Street name], [Number], [Floor] [Side]
[Postal code] [Locality]

Example:
Joao Silva
Rua Augusta, 25, 3o Esq.
1100-053 LISBOA
```

- **Postal code format:** `####-###` (4 digits, hyphen, 3 digits). The first 4 digits identify the area; the last 3 narrow to delivery point. Range: 1000-000 to 9980-999.
- **Street number:** After street name, separated by comma. Floor/side: `[floor]o [side]` (Esq.=Esquerdo/Left, Dto.=Direito/Right, Frente=Front).
- **Street types:** Rua (R.), Avenida (Av.), Praca (Pca.), Largo, Travessa (Tv.), Calcada, Estrada (Estr.), Alameda

### Phone Number Format

- **Country code:** +351
- **Format patterns:**
  - Landline: `+351 2## ### ###` (geographic: 21=Lisboa, 22=Porto, 23x-27x=other regions)
  - Mobile: `+351 9## ### ###` (prefixes: 91x, 92x, 93x, 96x)
  - Total digits: 9
- **Domestic format:** `### ### ###` (no leading 0)

### Name Ordering

- **given_first** (e.g., Joao Silva)

### Data Quality Assessment

- **Good.** GeoNames provides very good coverage (~34K entries). The 7-digit postal code system (####-###) is more granular than most European countries. Portuguese characters (a, a, c, e, o, o) need handling. The district system is well-established. CTT data availability has improved through open data initiatives.

---

## Greece (GR)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **ELTA (Hellenic Post)** | https://www.elta.gr/en-us/findapostcode.aspx | Web lookup | Official postal service. No free bulk download. |
| GeoNames | https://download.geonames.org/export/zip/GR.zip | TSV | ~5,500 entries. CC BY 4.0. |
| geodata.gov.gr | https://geodata.gov.gr/ | Various | Greek open geospatial data. |
| ELSTAT (Hellenic Statistical Authority) | https://www.statistics.gr/ | Various | Official statistical data. Administrative divisions. |

**Recommended primary source:** GeoNames for postal code data. ELSTAT for administrative geography.

### Administrative Divisions (Cities/States)

- **13 peripheries (regions)** + 1 autonomous monastic community:
  - Anatoliki Makedonia kai Thraki, Kentriki Makedonia, Dytiki Makedonia, Ipeiros, Thessalia, Sterea Ellada, Dytiki Ellada, Peloponnisos, Attiki, Voreio Aigaio, Notio Aigaio, Kriti, Ionioi Nisoi
  - Mount Athos (Agion Oros) -- autonomous
- **74 regional units** (periphereiakes enotites, replaced former prefectures in 2011 Kallikratis reform)
- **332 dimoi** (municipalities, reduced from ~1,000 in 2011 reform)
- Source: ELSTAT: https://www.statistics.gr/
- Wikipedia: https://en.wikipedia.org/wiki/Administrative_regions_of_Greece

### Address Format

```
[Recipient name]
[Street name] [Number]
[Postal code] [CITY]

Example (Latin script):
Giorgos Papadopoulos
Stadiou 25
105 59 ATHINA

Example (Greek script):
Γιώργος Παπαδόπουλος
Σταδίου 25
105 59 ΑΘΗΝΑ
```

- **Postal code format (TK - Tachydromikos Kodikas):** `### ##` (3 digits + space + 2 digits). Range: 100 00 - 854 00. First two digits indicate region (1x=Attiki/Athens, 2x=Central Greece/Peloponnese, 3x=Macedonia, 4x=Thessaly/Epirus, 5x=Macedonia/Thrace, 6x=Macedonia/Thrace, 7x=Crete, 8x=Aegean Islands).
- **Street number:** After street name. Typically 1-300.
- **Street types:** Odos (Οδός, street -- often omitted), Leoforos (Λεωφόρος, avenue), Plateia (Πλατεία, square)
- **Script:** Greek alphabet officially, but Latin transliteration common for international use.

### Phone Number Format

- **Country code:** +30
- **Format patterns:**
  - Landline: `+30 2## ### ####` (geographic, starts with 2; Athens: 210, Thessaloniki: 231x)
  - Mobile: `+30 69# ### ####` (prefixes: 690-699)
  - Total digits: 10
- **Domestic format:** `2## ### ####` or `69# ### ####` (no leading 0)

### Name Ordering

- **given_first** (e.g., Giorgos Papadopoulos / Γιώργος Παπαδόπουλος)

### Data Quality Assessment

- **Moderate.** GeoNames provides decent postal code coverage. The main challenge is dual-script handling (Greek and Latin). ELTA doesn't provide free bulk data. The 2011 Kallikratis administrative reform significantly changed municipal boundaries. Greek transliteration is not standardized (e.g., ELOT 743 vs. UN/UNGEGN systems), which can cause inconsistencies.

---

## Ireland (IE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Eircode** | https://www.eircode.ie/ | Proprietary | Official Irish postcode system (introduced 2015). Commercially licensed via Capita/Autoaddress. Not freely available in bulk. Format: A65 F4E2 (routing key + unique ID). |
| **GeoDirectory** | https://www.geodirectory.ie/ | Proprietary | Comprehensive Irish address database. Joint venture An Post/Ordnance Survey Ireland. Commercially licensed. |
| GeoNames | https://download.geonames.org/export/zip/IE.zip | TSV | ~5,000 entries. CC BY 4.0. Limited Eircode coverage (may use older routing key data). |
| data.gov.ie | https://data.gov.ie/ | Various | Irish open data portal. Some geographic datasets. |
| OSi (Ordnance Survey Ireland) | https://data-osi.opendata.arcgis.com/ | Various | Open map data. Administrative boundaries. |

**Recommended primary source:** GeoNames for basic coverage. For Eircodes, commercial license from Eircode/GeoDirectory required. OSi for boundary data.

### Administrative Divisions (Cities/States)

- **4 provinces** (historical, not administrative): Leinster, Munster, Connacht, Ulster (3 counties in Republic)
- **26 counties** (traditional) / **31 local authorities:**
  - Carlow, Cavan, Clare, Cork (city + county), Donegal, Dublin (city + 3 county councils: Dun Laoghaire-Rathdown, Fingal, South Dublin), Galway (city + county), Kerry, Kildare, Kilkenny, Laois, Leitrim, Limerick, Longford, Louth, Mayo, Meath, Monaghan, Offaly, Roscommon, Sligo, Tipperary, Waterford, Westmeath, Wexford, Wicklow
- Source: CSO (Central Statistics Office): https://www.cso.ie/
- Wikipedia: https://en.wikipedia.org/wiki/Counties_of_Ireland

### Address Format

```
[Recipient name]
[Building number/name] [Street name]
[Townland/Locality]
[Town/City]
[County]
[Eircode]

Example:
Sean Murphy
15 O'Connell Street
Dublin 1
D01 E4X0
```

- **Eircode format:** `A## A#A#` (3-character routing key + space + 4-character unique identifier). Routing key: letter + 2 digits (e.g., D01=Dublin 1, T12=Cork). Unique ID: mix of letters and digits. Introduced 2015.
- **Street number:** Before street name (British convention). Traditionally many rural Irish addresses had no street numbers -- just townland and locality names.
- **Street types:** Street, Road, Avenue, Lane, Place, Terrace, Drive, Park, Close, Way, Crescent
- **County:** Prefixed with "Co." (e.g., Co. Galway, Co. Kerry).

### Phone Number Format

- **Country code:** +353
- **Format patterns:**
  - Landline: `+353 1 ### ####` (Dublin) or `+353 ## ### ####` (other areas: 21=Cork, 61=Limerick, 91=Galway)
  - Mobile: `+353 8# ### ####` (prefixes: 083, 085, 086, 087, 089)
  - Total digits (excluding country code): 7-9 (variable)
- **Domestic format:** `0# ### ####` or `08# ### ####`

### Name Ordering

- **given_first** (e.g., Sean Murphy)

### Data Quality Assessment

- **Moderate.** Eircode is relatively new (2015) and commercially restricted. Ireland historically had weak postal code coverage. GeoNames data may not fully cover Eircodes. Rural addresses are notoriously unstructured (townland-based). For fake data generation, using Dublin/city addresses is more straightforward. The commercial restrictions on Eircode/GeoDirectory data are a significant limitation.

---

## Lithuania (LT)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Lietuvos Pastas (Lithuanian Post)** | https://www.post.lt/en/help/postal-codes | Web lookup | Official postal service. Some data downloadable. |
| GeoNames | https://download.geonames.org/export/zip/LT.zip | TSV | ~5,500 entries. CC BY 4.0. |
| Registru Centras (Register Centre) | https://www.registrucentras.lt/ | Various | Official address register. Comprehensive. |
| open.data.lt | https://data.gov.lt/ | Various | Lithuanian open data portal. |

**Recommended primary source:** GeoNames for postal code data. Registru Centras for official address data.

### Administrative Divisions (Cities/States)

- **10 apskritys (counties):** Alytaus, Kauno, Klaipedos, Marijampoles, Panevezio, Siauliu, Taurages, Telsiu, Utenos, Vilniaus
- **60 savivaldybes** (municipalities)
- Source: Statistics Lithuania: https://www.stat.gov.lt/
- Wikipedia: https://en.wikipedia.org/wiki/Counties_of_Lithuania

### Address Format

```
[Recipient name]
[Street name] [Number]-[Apartment]
[Postal code] [City]
[Municipality]

Example:
Jonas Kazlauskas
Gedimino pr. 15-7
LT-01103 Vilnius
```

- **Postal code format:** `LT-#####` (country prefix + 5 digits). Range: LT-01001 to LT-99069. First two digits indicate city/region (01-04=Vilnius, 44-49=Kaunas, 89-93=Klaipeda).
- **Street number:** After street name. Apartment: separated by hyphen (e.g., 15-7 = building 15, apartment 7).
- **Street types:** gatve (g., street), prospektas (pr., avenue/prospect), aikste (a., square), kelias (road), aleja (alley/avenue), skersgatvis (lane)

### Phone Number Format

- **Country code:** +370
- **Format patterns:**
  - Landline: `+370 # ### ####` (area codes: 5=Vilnius, 37=Kaunas, 46=Klaipeda)
  - Mobile: `+370 6## #####` (prefixes: 600-699)
  - Total digits (excluding country code): 8
- **Domestic format:** `(8-#) ### ####` or `8-6## #####`

### Name Ordering

- **given_first** (e.g., Jonas Kazlauskas)

### Data Quality Assessment

- **Moderate.** GeoNames provides reasonable postal code coverage. The LT- prefix is required in postal codes. Lithuanian characters (a, c, e, e, i, s, u, u, z) need handling. The Registru Centras has a comprehensive address register but access may be restricted. Overall data availability is adequate for fake data generation.

---

## Russia (RU)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Pochta Rossii (Russian Post)** | https://www.pochta.ru/post-index | Web lookup | Official postal service. Bulk data not freely downloadable. |
| **FIAS (Federal Address Information System)** | https://fias.nalog.ru/ | XML/DBF | Official Russian address database maintained by Federal Tax Service. Comprehensive. Free download. Very large dataset. |
| GeoNames | https://download.geonames.org/export/zip/RU.zip | TSV | ~14,400 entries. CC BY 4.0. |
| KLADR (Klassifikator Adresov Rossii) | Predecessor to FIAS | DBF | Legacy but still referenced. Being replaced by FIAS/GAR. |

**Recommended primary source:** FIAS/GAR for comprehensive official data. GeoNames for simpler postal code-place mapping.

### Administrative Divisions (Cities/States)

- **89 federal subjects** (as of current boundaries, including disputed territories):
  - 24 republics, 9 krais (territories), 48 oblasts (provinces), 3 federal cities (Moscow, St. Petersburg, Sevastopol*), 1 autonomous oblast (Jewish), 4 autonomous okrugs (districts)
  - *Sevastopol's status is disputed
- Major regions by type:
  - Federal cities: Moskva (Moscow), Sankt-Peterburg (St. Petersburg)
  - Oblasts: Moskovskaya, Leningradskaya, Sverdlovskaya, Novosibirskaya, etc.
  - Republics: Tatarstan, Bashkortostan, Dagestan, Chechnya, etc.
  - Krais: Krasnodarsky, Krasnoyarsky, Permsky, etc.
- **~22,000 municipalities and urban/rural settlements**
- Source: Rosstat (Federal State Statistics Service): https://rosstat.gov.ru/
- Wikipedia: https://en.wikipedia.org/wiki/Federal_subjects_of_Russia

### Address Format

```
[Country (for international)]
[Postal code], [Oblast/Republic/Krai]
[City/Settlement]
[Street type] [Street name], d. [Building], kv. [Apartment]
[Recipient name]

Example (Russian convention -- reverse order from Western):
123456, Moskovskaya obl.
g. Moskva
ul. Tverskaya, d. 15, kv. 7
Ivanov Ivan Petrovich

Example (Western-adapted):
Ivan Ivanov
ul. Tverskaya, d. 15, kv. 7
123456 Moscow
```

- **Postal code format (indeks):** `######` (6 digits). Range: 101000-999999. First 3 digits indicate region/city (101-135=Moscow, 190-199=St. Petersburg, 600-641=Nizhny Novgorod area).
- **Street number:** `d.` (dom=building/house). Apartment: `kv.` (kvartira). Building section: `korp.` (korpus). Structure: `str.` (stroenie).
- **Street types:** ulitsa (ul., street), prospekt (pr., avenue/prospect), pereulok (per., lane), bulvar (b-r, boulevard), ploshchad (pl., square), shosse (highway), naberezhnaya (nab., embankment), proezd (passage)
- **Name order in address:** Traditionally surname-first in official contexts (Ivanov Ivan Petrovich), but given_first in casual use.
- **Address order:** Russian convention is large-to-small (country, postal code, region, city, street, name) -- reverse of Western order.

### Phone Number Format

- **Country code:** +7
- **Format patterns:**
  - Landline: `+7 ### ### ## ##` (area codes: 495/499=Moscow, 812=St. Petersburg, 343=Yekaterinburg, 383=Novosibirsk)
  - Mobile: `+7 9## ### ## ##` (prefixes: 900-999, e.g., 903, 916, 926, 985=Moscow MTS; 901, 951=Moscow Beeline)
  - Total digits (excluding country code): 10
- **Domestic format:** `8 (###) ###-##-##` (8 replaces +7 for domestic calls)

### Name Ordering

- **given_first** in everyday use (e.g., Ivan Ivanov)
- Note: Official documents use family_first with patronymic (Ivanov Ivan Petrovich). For fake data generation, given_first is standard for Western-format output.

### Data Quality Assessment

- **Moderate.** FIAS/GAR is comprehensive but extremely large and complex (the full database is many gigabytes). GeoNames provides reasonable postal code coverage for major areas. The 6-digit postal code system is straightforward. Cyrillic script handling is essential. Transliteration varies (ISO 9, GOST, BGN/PCGN systems). Russia's vast size means many rural postcodes. The address format differs significantly from Western conventions (reverse order, patronymics, building/corpus/structure notation).

---

## Summary Comparison Table

| Country | Code | Postal Format | Digits | Free Official Data | GeoNames Entries | Address Complexity |
|---------|------|---------------|--------|-------------------|------------------|--------------------|
| UK | GB | A#[#] #AA | 5-7 chars | Yes (Code-Point Open) | ~27,000 | Medium (variable format) |
| France | FR | ##### | 5 | Yes (La Poste/data.gouv.fr) | ~51,000 | Low |
| Germany | DE | ##### | 5 | No (commercial) | ~16,400 | Low |
| Spain | ES | ##### | 5 | No | ~37,000 | Medium (floor/door) |
| Italy | IT | ##### | 5 | No | ~18,500 | Low-Medium |
| Netherlands | NL | #### AA | 6 chars | Yes (BAG/Kadaster) | ~4,600 | Medium (suffixes) |
| Belgium | BE | #### | 4 | No | ~2,700 | Medium (bilingual) |
| Switzerland | CH | #### | 4 | Yes (Swiss Post) | ~4,200 | Medium (multilingual) |
| Austria | AT | #### | 4 | Yes (data.gv.at) | ~2,500 | Low |
| Sweden | SE | ### ## | 5 | No | ~17,400 | Low |
| Norway | NO | #### | 4 | Yes (Posten/Bring) | ~4,700 | Low |
| Denmark | DK | #### | 4 | Yes (DAWA API) | ~1,200 | Medium (floor/side) |
| Finland | FI | ##### | 5 | Yes (Posti) | ~3,500 | Medium (staircase) |
| Poland | PL | ##-### | 5 | No | ~38,700 | Low-Medium |
| Czech Republic | CZ | ### ## | 5 | Yes (RUIAN) | ~16,700 | Medium (dual numbering) |
| Portugal | PT | ####-### | 7 | Partial | ~34,500 | Medium (floor/side) |
| Greece | GR | ### ## | 5 | No | ~5,500 | Medium (dual script) |
| Ireland | IE | A## A#A# | 7 chars | No (commercial) | ~5,000 | High (Eircode, rural) |
| Lithuania | LT | LT-##### | 5+prefix | No | ~5,500 | Low |
| Russia | RU | ###### | 6 | Yes (FIAS) | ~14,400 | High (Cyrillic, reverse order) |

## Key Findings

### Best Open Data Availability
1. **Denmark** -- DAWA API is world-class, free, comprehensive
2. **Norway** -- Posten/Bring provides free official postal code downloads
3. **Finland** -- Posti provides free official postal code downloads
4. **Switzerland** -- Swiss Post open data portal with multilingual data
5. **Netherlands** -- BAG (Kadaster) is one of the world's best open address databases
6. **France** -- La Poste/data.gouv.fr excellent open data ecosystem
7. **UK** -- Code-Point Open and ONSPD freely available
8. **Czech Republic** -- RUIAN official address register is freely downloadable

### Challenges by Country
- **Multilingual:** Belgium (NL/FR/DE), Switzerland (DE/FR/IT/RM), Finland (FI/SE)
- **Special characters:** All countries, but especially Poland, Czech Republic, Lithuania, and Scandinavian countries
- **Dual script:** Greece (Greek/Latin), Russia (Cyrillic/Latin)
- **Complex addressing:** Russia (reverse order, patronymics), Ireland (rural townlands, new Eircodes), Czech Republic (dual house numbering)
- **Commercial restrictions:** UK (Royal Mail PAF), Germany (Deutsche Post), Ireland (Eircode), Netherlands (PostNL bulk data)

### Universal Source: GeoNames
- Available for all 20 countries at https://download.geonames.org/export/zip/
- TSV format, CC BY 4.0 license
- Quality varies: excellent for France, Poland, Portugal; adequate for smaller countries
- Should be used as baseline with country-specific sources for enrichment
