# Geo Data Research: Asia & Oceania

Research findings for generating fake geographic data for Asia and Oceania countries.
Covers postal codes, administrative divisions, address formats, phone formats, and name ordering.

---

## Table of Contents

1. [Japan (JP)](#japan-jp)
2. [South Korea (KR)](#south-korea-kr)
3. [China (CN)](#china-cn)
4. [India (IN)](#india-in)
5. [Singapore (SG)](#singapore-sg)
6. [Australia (AU)](#australia-au)
7. [New Zealand (NZ)](#new-zealand-nz)
8. [Thailand (TH)](#thailand-th)
9. [Indonesia (ID)](#indonesia-id)
10. [Vietnam (VN)](#vietnam-vn)
11. [Philippines (PH)](#philippines-ph)
12. [Malaysia (MY)](#malaysia-my)
13. [Taiwan (TW)](#taiwan-tw)

---

## Japan (JP)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Japan Post (official)** | https://www.post.japanpost.jp/zipcode/download.html | CSV (Shift_JIS encoding) | Official source. Updated monthly. ~124,000 entries. Free download. Includes prefecture, city, and town names in both kanji and katakana. |
| GeoNames | https://download.geonames.org/export/zip/JP.zip | TSV | ~146,000 entries. Romanized place names. CC BY 4.0 license. |
| Datahub.io | https://datahub.io/core/postal-codes-jp | CSV/JSON | Derived from Japan Post data. |

**Recommended primary source:** Japan Post official data for completeness; GeoNames for romanized names.

### Administrative Divisions (Cities/States)

- **47 prefectures** (todofuken): 1 metropolis (to: Tokyo), 1 circuit (do: Hokkaido), 2 urban prefectures (fu: Osaka, Kyoto), 43 prefectures (ken)
- Source: https://www.soumu.go.jp/ (Ministry of Internal Affairs) publishes the official list with JIS X 0401 codes
- GeoNames admin1 codes cover all 47 prefectures
- Wikipedia: https://en.wikipedia.org/wiki/Prefectures_of_Japan (good reference for native names)

### Address Format

```
〒###-####
[Prefecture] [City/Ward] [Town] [Block]-[Building number]
[Building name] [Room number]
[Recipient name] 様

Example:
〒100-0001
東京都千代田区千代田1-1
皇居
天皇陛下
```

- **Order:** Large to small (prefecture -> city -> district -> block -> building)
- **Postal code format:** `###-####` (3 digits, hyphen, 4 digits)
- Street numbers: `[chome]-[banchi]-[go]` pattern (e.g., `1-2-3`)
- Most streets are unnamed; addresses use block/lot numbering
- Building number typically 1-50 range

### Phone Number Format

- **Country code:** +81
- **Format patterns:**
  - Landline: `+81-#-####-####` (Tokyo area code 3), `+81-##-###-####` (other areas)
  - Mobile: `+81-##-####-####` (prefixes: 070, 080, 090)
  - General pattern: 10-11 digits total including area code
- **Domestic format:** `0#-####-####` or `0##-###-####`

### Name Ordering

- **family_first** (e.g., 田中 太郎 = Tanaka Taro)
- Family name (sei/myoji) followed by given name (mei)
- Common surname databases: https://myoji-yurai.net/

### Data Quality Assessment

- **Excellent.** Japan Post provides the most comprehensive and frequently updated postal code database of any country in this list. Data is authoritative and covers every deliverable address. The main challenge is Shift_JIS encoding and kanji/katakana dual representation.

---

## South Korea (KR)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Korea Post (official)** | https://www.epost.go.kr/search/zipcode/areacdAddressDown.jsp | Excel/CSV | Official 5-digit postal codes (switched from 6-digit in 2015). ~45,000 entries. Korean language. |
| GeoNames | https://download.geonames.org/export/zip/KR.zip | TSV | Romanized names. Good coverage. |
| Korean address system (JUSO) | https://www.juso.go.kr/ | Various | Road name address (도로명주소) data. Very comprehensive government portal. |

**Recommended primary source:** JUSO (Korean Address System) for the most comprehensive data; GeoNames for romanized versions.

### Administrative Divisions (Cities/States)

- **17 first-level divisions:** 1 special city (Seoul), 6 metropolitan cities (Busan, Daegu, Incheon, Gwangju, Daejeon, Ulsan), 1 special autonomous city (Sejong), 8 provinces (do), 1 special autonomous province (Jeju)
- Source: https://www.juso.go.kr/ — official administrative district data
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_South_Korea

### Address Format

```
[Province/City] [District/City] [Road name] [Building number]
[Detail address (dong, floor, unit)]
[Postal code]

Example:
서울특별시 중구 세종대로 110
(태평로1가)
04524
```

- **Order:** Large to small (province -> city -> road -> building number)
- **Road name address system** (도로명주소) adopted in 2014, replacing lot-number (지번) system
- Building numbers are sequential along roads (odd/even on each side)
- Postal code format: `#####` (5 digits, no separator)

### Phone Number Format

- **Country code:** +82
- **Format patterns:**
  - Landline: `+82-#-####-####` (Seoul: area code 2), `+82-##-###-####` or `+82-##-####-####`
  - Mobile: `+82-1#-####-####` (prefixes: 010, formerly 011, 016, 017, 018, 019)
- **Domestic format:** `0#-####-####` or `0##-####-####`

### Name Ordering

- **family_first** (e.g., 김 민수 = Kim Minsu)
- Family name (seong) followed by given name (ireum)
- ~280 Korean surnames; top 5 (Kim, Lee, Park, Choi, Jung) cover ~50% of population
- Common surname source: Korean Statistical Information Service (KOSIS)

### Data Quality Assessment

- **Excellent.** The JUSO system is one of the most modern and comprehensive address systems in the world. The transition to road-name addresses is complete. Data is freely available from government portals. GeoNames coverage is solid for romanized data.

---

## China (CN)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| GeoNames | https://download.geonames.org/export/zip/CN.zip | TSV | ~77,000 entries. Romanized (pinyin) names. |
| China Post (official) | https://www.chinapost.com.cn/ | Web lookup only | No bulk download available. 6-digit codes. |
| Community datasets | https://github.com/xiangyuecn/AreaCity-JsSpider-StatsGov | JSON/CSV | Scraped from National Bureau of Statistics. Very comprehensive for admin divisions. |
| Datahub.io | https://datahub.io/core/postal-codes-cn | CSV | Derived dataset. |

**Recommended primary source:** GeoNames for postal codes; xiangyuecn/AreaCity-JsSpider-StatsGov GitHub repo for administrative divisions.

### Administrative Divisions (Cities/States)

- **34 provincial-level divisions:** 23 provinces, 4 municipalities (Beijing, Shanghai, Tianjin, Chongqing), 5 autonomous regions, 2 SARs (Hong Kong, Macau)
- **333 prefecture-level divisions**, **2,844 county-level divisions**
- Source: National Bureau of Statistics (http://www.stats.gov.cn/sj/tjbz/tjyqhdmhcxhfdm/) publishes official administrative division codes
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_China

### Address Format

```
[Province] [City] [District] [Street] [Number]号
[Building name] [Unit] [Room]
[Postal code]

Example:
北京市东城区长安街1号
100000
```

- **Order:** Large to small (province -> city -> district -> street -> number)
- Postal code format: `######` (6 digits, no separator)
- First two digits indicate province, next two indicate city, last two indicate district
- Street numbers followed by 号 (hao)
- Building/floor/unit: 栋 (building), 楼/层 (floor), 室/号 (room)

### Phone Number Format

- **Country code:** +86
- **Format patterns:**
  - Landline: `+86-###-########` or `+86-####-#######` (area codes 2-4 digits, local 7-8 digits; total 10-12 digits with area code)
  - Mobile: `+86-###-####-####` (11 digits; prefixes: 13x, 14x, 15x, 16x, 17x, 18x, 19x)
- **Domestic format:** `0###-########` (landline), `1##-####-####` (mobile)

### Name Ordering

- **family_first** (e.g., 王 伟 = Wang Wei)
- Family name (xing) followed by given name (ming)
- ~6,000 surnames in use; top 100 cover ~85% of population
- Given names are 1-2 characters
- Source for common names: Ministry of Public Security annual name reports

### Data Quality Assessment

- **Moderate.** China Post does not provide a bulk download, making GeoNames the best freely available option. However, GeoNames coverage may be incomplete for rural areas. The GitHub community datasets for administrative divisions are very good. Postal codes are less standardized than in Japan or Korea — some rural areas share codes across large regions.

---

## India (IN)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **India Post (official)** | https://data.gov.in/resource/all-india-pincode-directory | CSV | Official PIN code directory. ~155,000 post offices with PIN codes. Open Government Data (OGD) Platform. |
| GeoNames | https://download.geonames.org/export/zip/IN.zip | TSV | Good coverage. Romanized names. |
| Datahub.io | https://datahub.io/core/postal-codes-in | CSV | Derived from official data. |

**Recommended primary source:** India Post via data.gov.in — it is the most comprehensive and authoritative source.

### Administrative Divisions (Cities/States)

- **28 states and 8 union territories** (as of 2024)
- Source: https://data.gov.in/ — official government data portal
- Census of India provides detailed district and sub-district data
- Wikipedia: https://en.wikipedia.org/wiki/States_and_union_territories_of_India
- ~780 districts, ~5,900+ sub-districts

### Address Format

```
[Recipient name]
[House/Flat number], [Building name]
[Street/Road name]
[Area/Locality]
[City] - [PIN code]
[State]

Example:
Mr. Rajesh Kumar
42, Sunshine Apartments
MG Road, Indiranagar
Bangalore - 560038
Karnataka
```

- **Order:** Small to large (building -> street -> area -> city -> state) — Western-style
- PIN code format: `######` (6 digits, no separator)
- First digit indicates region (1-9), second digit indicates sub-region, third digit indicates sorting district
- PIN = Postal Index Number
- House numbers can be alphanumeric (e.g., `12/A`, `3-45-67`)

### Phone Number Format

- **Country code:** +91
- **Format patterns:**
  - Landline: `+91-##-########` or `+91-###-#######` (area code 2-4 digits, local 6-8 digits; total 10 digits)
  - Mobile: `+91-#####-#####` (10 digits; prefixes: 6, 7, 8, 9)
- **Domestic format:** `0##-########` (landline), `#####-#####` (mobile, no leading 0)

### Name Ordering

- **given_first** (generally) — but highly variable by region and culture
- North India: Given name + surname (e.g., Rajesh Kumar)
- South India: Often initial-based (e.g., K. Rajesh where K = father's name initial)
- Some regions use patronymics rather than family surnames
- Source for common names: Census name frequency data

### Data Quality Assessment

- **Good.** India's open data portal provides comprehensive PIN code data. The main challenge is the sheer diversity of address formats across India — there is no single standardized format. Transliteration between multiple scripts (Hindi, Tamil, Bengali, etc.) adds complexity. GeoNames data is solid for major areas but may miss very rural post offices.

---

## Singapore (SG)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Singapore Post / URA** | https://data.gov.sg/ | CSV/API | Government open data. Singapore uses 6-digit postal codes, one per building (unique per address). |
| GeoNames | https://download.geonames.org/export/zip/SG.zip | TSV | Limited — Singapore is small with ~150,000+ unique postal codes. |
| OneMap API | https://www.onemap.gov.sg/apidocs/ | REST API | Singapore Land Authority. Comprehensive geocoding with postal codes. |

**Recommended primary source:** OneMap API for most complete data; data.gov.sg for bulk datasets.

### Administrative Divisions (Cities/States)

- Singapore is a **city-state** — no provinces or states
- **5 CDC districts** (Community Development Councils): Central, North East, North West, South East, South West
- **55 planning areas** defined by URA (Urban Redevelopment Authority)
- Source: https://data.gov.sg/ and URA Master Plan
- Wikipedia: https://en.wikipedia.org/wiki/Planning_Areas_of_Singapore

### Address Format

```
[Block number] [Street name]
[Unit number: #floor-unit]
[Building name (if applicable)]
Singapore [6-digit postal code]

Example:
Blk 123 Ang Mo Kio Avenue 6
#08-123
Singapore 560123
```

- **Order:** Small to large (block -> street -> unit -> postal code)
- Postal code format: `######` (6 digits, unique per building in most cases)
- Block numbers: typically 1-999
- Unit format: `#[floor]-[unit]` (e.g., `#08-123`)
- HDB (public housing) vs private address formats differ slightly

### Phone Number Format

- **Country code:** +65
- **Format patterns:**
  - Landline: `+65-6###-####` (8 digits, starts with 6)
  - Mobile: `+65-8###-####` or `+65-9###-####` (8 digits, starts with 8 or 9)
- **No area codes** — Singapore is too small
- **Domestic format:** `####-####` (8 digits)

### Name Ordering

- **Mixed** — depends on ethnicity:
  - Chinese names: **family_first** (e.g., Tan Ah Kow)
  - Malay names: **given_first** (e.g., Muhammad bin Abdullah)
  - Indian names: **given_first** (e.g., Rajesh s/o Kumar)
- For fake data generation, recommend defaulting to **family_first** (Chinese naming is most common, ~75% of population)

### Data Quality Assessment

- **Excellent.** Singapore has one of the best open data ecosystems in Asia. The OneMap API is comprehensive and well-maintained. The small geographic size means complete coverage is achievable. Data is high quality and frequently updated.

---

## Australia (AU)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Australia Post (official)** | https://auspost.com.au/business/marketing/create-online-communications/access-data-and-insights/address-data | CSV | Official postcode data. ~16,000 localities. Free for non-commercial use (license restrictions for commercial). |
| GeoNames | https://download.geonames.org/export/zip/AU.zip | TSV | ~16,000 entries. Good coverage. CC BY 4.0. |
| data.gov.au | https://data.gov.au/ | CSV | Various government datasets including postcodes. |
| Matthew Proctor's dataset | https://www.matthewproctor.com/australian_postcodes | CSV | Community-maintained, frequently updated, freely licensed. ~18,000 entries with lat/long. |

**Recommended primary source:** GeoNames for permissive licensing; Matthew Proctor dataset for most complete free data; Australia Post for authoritative data (check license).

### Administrative Divisions (Cities/States)

- **6 states:** New South Wales (NSW), Victoria (VIC), Queensland (QLD), South Australia (SA), Western Australia (WA), Tasmania (TAS)
- **2 mainland territories:** Australian Capital Territory (ACT), Northern Territory (NT)
- **Other territories:** Norfolk Island, Christmas Island, Cocos Islands, etc.
- Source: Australian Bureau of Statistics (ABS) — https://www.abs.gov.au/
- Wikipedia: https://en.wikipedia.org/wiki/States_and_territories_of_Australia

### Address Format

```
[Recipient name]
[Unit/Level number] [Street number] [Street name] [Street type]
[Suburb/Locality] [State abbreviation] [Postcode]

Example:
Mr John Smith
Unit 5, 123 Collins Street
Melbourne VIC 3000
```

- **Order:** Small to large (unit -> street -> suburb -> state -> postcode)
- Postcode format: `####` (4 digits)
  - 0xxx: NT
  - 1xxx: NSW (PO boxes)
  - 2xxx: NSW, ACT
  - 3xxx: VIC
  - 4xxx: QLD
  - 5xxx: SA
  - 6xxx: WA
  - 7xxx: TAS
- Street types: Street, Road, Avenue, Drive, Place, Court, Crescent, etc.
- Unit format: `Unit #`, `Apt #`, `Level #`, `Suite #`

### Phone Number Format

- **Country code:** +61
- **Format patterns:**
  - Landline: `+61-#-####-####` (area code 1 digit: 2=NSW/ACT, 3=VIC/TAS, 7=QLD, 8=SA/WA/NT)
  - Mobile: `+61-4##-###-###` (starts with 04, 10 digits)
- **Domestic format:** `(0#) ####-####` (landline), `04##-###-###` (mobile)

### Name Ordering

- **given_first** (e.g., John Smith)
- Western naming convention
- Source for common names: ABS Census name data

### Data Quality Assessment

- **Excellent.** Australia has well-maintained, freely available postal code data from multiple sources. The address format is straightforward and well-standardized. Australia Post data is authoritative but has licensing restrictions. GeoNames and community datasets provide good alternatives.

---

## New Zealand (NZ)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **NZ Post (official)** | https://www.nzpost.co.nz/business/sending-within-nz/addressing-and-postcode-finder | Web lookup | NZ Post provides a postcode finder. Bulk download requires a data license. |
| GeoNames | https://download.geonames.org/export/zip/NZ.zip | TSV | ~1,800 entries. Good coverage for NZ's size. |
| data.govt.nz | https://data.govt.nz/ | Various | NZ government open data portal. |
| Koordinates | https://koordinates.com/ | Various | NZ geospatial data platform with address data. |
| LINZ Data Service | https://data.linz.govt.nz/ | CSV/Shapefile | Land Information NZ. Includes NZ Address data (~2M addresses). CC BY 4.0. |

**Recommended primary source:** GeoNames for postal codes; LINZ Data Service for comprehensive address data.

### Administrative Divisions (Cities/States)

- **16 regions:** Northland, Auckland, Waikato, Bay of Plenty, Gisborne, Hawke's Bay, Taranaki, Manawatu-Whanganui, Wellington, Tasman, Nelson, Marlborough, West Coast, Canterbury, Otago, Southland
- **67 territorial authorities** (cities and districts)
- Source: Stats NZ — https://www.stats.govt.nz/
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_New_Zealand

### Address Format

```
[Recipient name]
[Unit number]/[Street number] [Street name]
[Suburb]
[City] [Postcode]

Example:
Jane Doe
2/45 Lambton Quay
Thorndon
Wellington 6011
```

- **Order:** Small to large (unit -> street -> suburb -> city -> postcode)
- Postcode format: `####` (4 digits)
- Unit numbers use slash notation: `2/45` means unit 2 at number 45
- Street types: Street, Road, Avenue, Drive, Place, Terrace, etc.

### Phone Number Format

- **Country code:** +64
- **Format patterns:**
  - Landline: `+64-#-###-####` (area code 1 digit: 3=South Island, 4=North Island lower, 6=Manawatu, 7=Waikato/BOP, 9=Auckland)
  - Mobile: `+64-2#-###-####` or `+64-2#-####-####` (starts with 02, 8-10 digits after prefix)
- **Domestic format:** `(0#) ###-####` (landline), `02#-###-####` (mobile)

### Name Ordering

- **given_first** (e.g., Jane Smith)
- Western naming convention
- Maori names may include macrons (e.g., Aroha Tamati)

### Data Quality Assessment

- **Good.** GeoNames provides reasonable postal code coverage. LINZ Data Service is excellent for comprehensive address data with permissive licensing. NZ Post bulk data requires licensing. Overall data availability is solid for a smaller country.

---

## Thailand (TH)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Thailand Post (official)** | https://www.thailandpost.co.th/ | Web lookup | Official postal code lookup. No bulk download readily available. |
| GeoNames | https://download.geonames.org/export/zip/TH.zip | TSV | ~2,600 entries. Romanized names. |
| Wikipedia | https://en.wikipedia.org/wiki/List_of_postal_codes_in_Thailand | HTML table | Complete list of Thai postal codes by province. Useful for scraping. |

**Recommended primary source:** GeoNames for bulk data; Wikipedia for verification.

### Administrative Divisions (Cities/States)

- **76 provinces** (changwat) + 1 special administrative area (Bangkok)
- **878 districts** (amphoe), **7,255 sub-districts** (tambon)
- Source: Department of Provincial Administration (DOPA) — https://www.dopa.go.th/
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_Thailand

### Address Format

```
[House number] [Village/Moo] [Soi (lane)]
[Road name]
[Sub-district (Tambon)] [District (Amphoe)]
[Province (Changwat)] [Postal code]

Example:
123 หมู่ 4 ซอยสุขุมวิท 55
ถนนสุขุมวิท
แขวงคลองตันเหนือ เขตวัฒนา
กรุงเทพมหานคร 10110

Romanized:
123 Moo 4, Soi Sukhumvit 55
Sukhumvit Road
Khlong Tan Nuea, Watthana
Bangkok 10110
```

- **Order:** Small to large (house -> soi -> road -> sub-district -> district -> province -> postcode)
- Postal code format: `#####` (5 digits)
- First two digits indicate province
- Bangkok addresses use Khet (เขต) / Khwaeng (แขวง) instead of Amphoe/Tambon
- Soi (ซอย) = lane/alley, very common in addressing

### Phone Number Format

- **Country code:** +66
- **Format patterns:**
  - Landline: `+66-#-###-####` (Bangkok: area code 2), `+66-##-###-####` (other areas)
  - Mobile: `+66-##-###-####` (10 digits; prefixes: 06, 08, 09)
- **Domestic format:** `0#-###-####` or `0##-###-####` (landline), `0##-###-####` (mobile)

### Name Ordering

- **given_first** (e.g., Somchai Jaidee)
- Given name (chue) followed by family name (naamsagul)
- Thai surnames were mandated in 1913; each surname is unique to one family
- Thais are typically addressed by their given name, not surname
- Nicknames (chue len) are very common and often used in daily life

### Data Quality Assessment

- **Moderate.** Thailand Post does not offer bulk downloads. GeoNames provides the best freely available postal code data but may have gaps in rural areas. Administrative division data is well-documented. The Thai script adds a localization challenge; romanization follows RTGS (Royal Thai General System) but is inconsistent in practice.

---

## Indonesia (ID)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Pos Indonesia (official)** | https://www.posindonesia.co.id/ | Web lookup | Official lookup, no bulk download. |
| GeoNames | https://download.geonames.org/export/zip/ID.zip | TSV | ~7,300 entries. Romanized names. |
| Kodepos.id | https://kodepos.id/ | Web | Community-maintained postal code directory. |
| Wikipedia | https://en.wikipedia.org/wiki/List_of_postal_codes_in_Indonesia | HTML table | Organized by province. |

**Recommended primary source:** GeoNames for bulk data.

### Administrative Divisions (Cities/States)

- **38 provinces** (provinsi) as of 2024 (several new provinces created from splits)
- **514 regencies** (kabupaten) and **98 cities** (kota)
- Source: BPS (Statistics Indonesia) — https://www.bps.go.id/
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_Indonesia

### Address Format

```
[Street name] No. [Number]
RT [number]/RW [number]
[Kelurahan/Desa], [Kecamatan]
[Kota/Kabupaten], [Province] [Postal code]

Example:
Jl. Sudirman No. 45
RT 003/RW 007
Kelurahan Senayan, Kecamatan Kebayoran Baru
Jakarta Selatan, DKI Jakarta 12190
```

- **Order:** Small to large (street -> neighborhood -> village -> district -> city -> province -> postcode)
- Postal code format: `#####` (5 digits)
- `Jl.` or `Jln.` = Jalan (street)
- RT/RW = neighborhood/community association numbers
- House numbers follow `No.` prefix

### Phone Number Format

- **Country code:** +62
- **Format patterns:**
  - Landline: `+62-##-###-####` or `+62-###-###-####` (area codes 2-3 digits)
  - Mobile: `+62-8##-####-####` (10-12 digits; always starts with 8 after country code)
- **Domestic format:** `(0##) ###-####` (landline), `08##-####-####` (mobile)

### Name Ordering

- **given_first** (generally)
- Many Indonesians use a single name (mononym), especially Javanese (e.g., Suharto, Sukarno)
- Others use Western-style given + family name
- Highly variable by ethnic group (Javanese, Sundanese, Batak, etc.)

### Data Quality Assessment

- **Moderate.** Indonesia's postal code system covers a vast archipelago of 17,000+ islands. GeoNames has reasonable coverage. No official bulk download exists. The address system with RT/RW is unique and not always consistently applied. Province boundaries are periodically redrawn, requiring data updates.

---

## Vietnam (VN)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Vietnam Post (official)** | https://vnpost.vn/ | Web lookup | Official lookup only. 6-digit postal codes (reformed ~2017). |
| GeoNames | https://download.geonames.org/export/zip/VN.zip | TSV | Coverage may be limited. |
| Wikipedia | https://en.wikipedia.org/wiki/Postal_codes_in_Vietnam | HTML table | Lists codes by province. Useful reference. |

**Recommended primary source:** GeoNames supplemented by Wikipedia reference tables.

### Administrative Divisions (Cities/States)

- **58 provinces** (tinh) + **5 centrally-controlled municipalities** (thanh pho truc thuoc trung uong): Hanoi, Ho Chi Minh City, Hai Phong, Da Nang, Can Tho
- **705 district-level divisions**, **10,600+ commune-level divisions**
- Source: General Statistics Office — https://www.gso.gov.vn/
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_Vietnam

### Address Format

```
[House number] [Street name]
[Ward (Phuong/Xa)], [District (Quan/Huyen)]
[Province/City]
[Postal code]

Example:
123 Nguyen Hue
Phuong Ben Nghe, Quan 1
Thanh pho Ho Chi Minh
700000
```

- **Order:** Small to large (number -> street -> ward -> district -> city/province -> postcode)
- Postal code format: `######` (6 digits)
- First two digits indicate province
- Street names often honor historical figures
- District numbers common in major cities (e.g., Quan 1, Quan 3)

### Phone Number Format

- **Country code:** +84
- **Format patterns:**
  - Landline: `+84-###-###-####` (area codes 2-3 digits, local 7-8 digits)
  - Mobile: `+84-##-####-####` (10 digits after 0; prefixes: 03, 05, 07, 08, 09)
- **Domestic format:** `0###-###-####` (landline), `0##-####-####` (mobile)

### Name Ordering

- **family_first** (e.g., Nguyen Van Minh)
- Structure: Family name + middle name + given name
- People are addressed by their given name (last element), not family name
- ~14 common surnames; Nguyen alone covers ~40% of population

### Data Quality Assessment

- **Moderate.** Vietnam reformed its postal code system from 5-digit to 6-digit codes around 2017. Data is still stabilizing. GeoNames coverage is improving but may lag behind the reforms. Official bulk data is not freely available. Administrative division data is well-documented but commune-level data is very large.

---

## Philippines (PH)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **PHLPost (official)** | https://www.phlpost.gov.ph/ | Web lookup / PDF | Official postal code directory. 4-digit codes. |
| GeoNames | https://download.geonames.org/export/zip/PH.zip | TSV | Coverage available. |
| Wikipedia | https://en.wikipedia.org/wiki/List_of_ZIP_codes_in_the_Philippines | HTML table | Comprehensive list by region. |

**Recommended primary source:** GeoNames for bulk data; Wikipedia for verification.

### Administrative Divisions (Cities/States)

- **17 regions** (administrative, not political), **82 provinces**, **146 cities**, **1,488 municipalities**
- Source: Philippine Statistics Authority (PSA) — https://psa.gov.ph/
- The Philippine Standard Geographic Code (PSGC) provides complete administrative division codes
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_the_Philippines

### Address Format

```
[House/Lot number] [Street name]
[Barangay (village)]
[City/Municipality], [Province] [Postal code]

Example:
123 Rizal Avenue
Barangay San Antonio
Makati City, Metro Manila 1200
```

- **Order:** Small to large (house -> street -> barangay -> city -> province -> postcode)
- Postal code format: `####` (4 digits)
- Barangay (smallest admin unit) is commonly included in addresses
- Metro Manila uses city-level addressing without province

### Phone Number Format

- **Country code:** +63
- **Format patterns:**
  - Landline: `+63-#-####-####` (Manila: area code 2, 8 digits) or `+63-##-###-####` (provincial)
  - Mobile: `+63-9##-###-####` (10 digits; starts with 9)
- **Domestic format:** `(0#) ####-####` (landline), `09##-###-####` (mobile)

### Name Ordering

- **given_first** (e.g., Juan Dela Cruz)
- Western naming convention (influenced by Spanish/American colonization)
- Many have Spanish-origin surnames
- Middle name is typically mother's maiden name

### Data Quality Assessment

- **Moderate.** PHLPost data exists but bulk access is limited. GeoNames provides reasonable coverage. The address system is relatively straightforward (Western-influenced). The barangay layer adds granularity but also complexity. The PSGC is a good authoritative source for administrative codes.

---

## Malaysia (MY)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Pos Malaysia (official)** | https://www.pos.com.my/ | Web lookup | Official lookup. 5-digit postal codes. |
| GeoNames | https://download.geonames.org/export/zip/MY.zip | TSV | Good coverage. |
| data.gov.my | https://data.gov.my/ | CSV | Malaysian government open data portal. |
| Wikipedia | https://en.wikipedia.org/wiki/Postal_codes_in_Malaysia | HTML table | By state. |

**Recommended primary source:** GeoNames for bulk data; data.gov.my for official datasets.

### Administrative Divisions (Cities/States)

- **13 states** (negeri) + **3 federal territories** (wilayah persekutuan: Kuala Lumpur, Putrajaya, Labuan)
- **Peninsular Malaysia:** Johor, Kedah, Kelantan, Melaka, Negeri Sembilan, Pahang, Perak, Perlis, Penang, Sabah, Sarawak, Selangor, Terengganu
- Source: Department of Statistics Malaysia — https://www.dosm.gov.my/
- Wikipedia: https://en.wikipedia.org/wiki/States_and_federal_territories_of_Malaysia

### Address Format

```
[House/Lot number], [Street name]
[Taman/Section/Area]
[Postal code] [City/Town]
[State]

Example:
No. 123, Jalan Bukit Bintang
Taman Sri Hartamas
50200 Kuala Lumpur
Wilayah Persekutuan
```

- **Order:** Small to large (house -> street -> area -> postcode+city -> state)
- Postal code format: `#####` (5 digits)
- Postcode comes BEFORE city name on the same line
- `Jalan` (Jln.) = street/road
- `Taman` = housing estate/garden, `Lorong` = lane
- `No.` prefix for house numbers

### Phone Number Format

- **Country code:** +60
- **Format patterns:**
  - Landline: `+60-#-####-####` (KL: area code 3) or `+60-##-###-####` (other areas)
  - Mobile: `+60-1#-###-####` or `+60-1##-###-####` (starts with 01, 9-10 digits)
- **Domestic format:** `0#-####-####` (landline), `01#-###-####` (mobile)

### Name Ordering

- **Mixed** — depends on ethnicity:
  - Malay: **given_first** with patronymic (e.g., Ahmad bin Abdullah; `bin` = son of, `binti` = daughter of)
  - Chinese: **family_first** (e.g., Tan Ah Kow)
  - Indian: **given_first** with patronymic (e.g., Rajesh a/l Kumar; `a/l` = son of, `a/p` = daughter of)
- For fake data generation, recommend defaulting to **given_first** (Malay naming is most common, ~60% of population)

### Data Quality Assessment

- **Good.** GeoNames provides solid coverage. The Malaysian government open data portal is improving. Address formats are well-standardized. The multi-ethnic naming conventions add complexity for fake data generation.

---

## Taiwan (TW)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Chunghwa Post (official)** | https://www.post.gov.tw/post/internet/Download/index.jsp?ID=220 | CSV/ZIP | Official postal code data. 3+2 digit system (3-digit for district, 5-digit for detailed). Free download. |
| GeoNames | https://download.geonames.org/export/zip/TW.zip | TSV | Coverage available. |
| Wikipedia | https://en.wikipedia.org/wiki/List_of_postal_codes_of_the_Republic_of_China | HTML table | By county/city. |

**Recommended primary source:** Chunghwa Post official data for comprehensive coverage; GeoNames for romanized data.

### Administrative Divisions (Cities/States)

- **6 special municipalities** (zhixiashi): Taipei, New Taipei, Taoyuan, Taichung, Tainan, Kaohsiung
- **3 provincial cities** (shengxiashi): Keelung, Hsinchu, Chiayi
- **13 counties** (xian)
- Source: Ministry of the Interior — https://www.moi.gov.tw/
- Wikipedia: https://en.wikipedia.org/wiki/Subdivisions_of_Taiwan

### Address Format

```
[Postal code]
[County/City] [District/Township] [Road/Street] [Section]段 [Lane]巷 [Alley]弄 [Number]號 [Floor]樓

Example:
100
台北市中正區重慶南路一段122號

Romanized:
100
Taipei City, Zhongzheng District, Sec. 1, Chongqing South Road, No. 122
```

- **Order:** Large to small (city -> district -> road -> section -> lane -> alley -> number -> floor)
- Postal code format: `###` (3 digits basic) or `###-##` (5 digits detailed)
- Section (段, duan), Lane (巷, xiang), Alley (弄, nong) provide progressive narrowing
- Number followed by 號 (hao)
- Floor indicated by 樓 (lou) — e.g., `5樓` = 5th floor

### Phone Number Format

- **Country code:** +886
- **Format patterns:**
  - Landline: `+886-#-####-####` (Taipei: area code 2) or `+886-##-###-####` (other areas)
  - Mobile: `+886-9##-###-###` (9 digits; starts with 09)
- **Domestic format:** `(0#) ####-####` (landline), `09##-###-###` (mobile)

### Name Ordering

- **family_first** (e.g., 陳 志明 = Chen Zhiming)
- Same convention as mainland China
- Family name (xing) followed by given name (ming)
- Given names typically 1-2 characters

### Data Quality Assessment

- **Excellent.** Chunghwa Post provides a comprehensive, freely downloadable postal code database. Taiwan has a well-organized address system. GeoNames coverage is solid. The 3+2 digit postal code system is well-documented. Data quality is comparable to Japan and South Korea.

---

## Cross-Country Summary

### Primary Data Source Recommendations

| Country | Postal Codes | Admin Divisions | Best Overall Source |
|---------|-------------|-----------------|---------------------|
| JP | Japan Post (official) | GeoNames / SOUMU | Japan Post + GeoNames |
| KR | JUSO / Korea Post | JUSO | JUSO (juso.go.kr) |
| CN | GeoNames | NBS / GitHub community | GeoNames + community repos |
| IN | data.gov.in (India Post) | Census / data.gov.in | data.gov.in |
| SG | OneMap API | data.gov.sg | OneMap + data.gov.sg |
| AU | GeoNames / Australia Post | ABS | GeoNames |
| NZ | GeoNames / LINZ | Stats NZ | LINZ Data Service |
| TH | GeoNames | DOPA | GeoNames |
| ID | GeoNames | BPS | GeoNames |
| VN | GeoNames | GSO | GeoNames |
| PH | GeoNames | PSA (PSGC) | GeoNames + PSGC |
| MY | GeoNames | DOSM | GeoNames |
| TW | Chunghwa Post | MOI | Chunghwa Post + GeoNames |

### GeoNames as Universal Fallback

GeoNames (https://download.geonames.org/export/zip/) provides postal code data for **all 13 countries** in this list. The data is:
- Licensed under CC BY 4.0 (permissive)
- Available in a consistent TSV format
- Includes romanized place names
- Updated regularly by community contributions
- Fields: country code, postal code, place name, admin name 1-3, admin code 1-3, latitude, longitude, accuracy

**Recommendation:** Use GeoNames as the universal data source for initial implementation, then enhance with country-specific official sources where higher quality is needed (especially JP, KR, SG, TW, AU).

### Name Ordering Summary

| Ordering | Countries |
|----------|-----------|
| **family_first** | JP, KR, CN, TW, VN |
| **given_first** | IN (mostly), AU, NZ, TH, ID (mostly), PH |
| **mixed** (depends on ethnicity) | SG, MY |

### Phone Format Summary

| Country | Code | Mobile Pattern | Digits (total) |
|---------|------|---------------|----------------|
| JP | +81 | 0[789]0-####-#### | 11 |
| KR | +82 | 010-####-#### | 11 |
| CN | +86 | 1##-####-#### | 11 |
| IN | +91 | [6-9]####-##### | 10 |
| SG | +65 | [89]###-#### | 8 |
| AU | +61 | 04##-###-### | 10 |
| NZ | +64 | 02#-###-####(#) | 8-10 |
| TH | +66 | 0[689]#-###-#### | 10 |
| ID | +62 | 08##-####-####(#) | 10-12 |
| VN | +84 | 0[35789]#-####-#### | 10 |
| PH | +63 | 09##-###-#### | 10 |
| MY | +60 | 01#-###-####(#) | 9-10 |
| TW | +886 | 09##-###-### | 9 |

### Postal Code Format Summary

| Country | Format | Digits | Example |
|---------|--------|--------|---------|
| JP | ###-#### | 7 | 100-0001 |
| KR | ##### | 5 | 04524 |
| CN | ###### | 6 | 100000 |
| IN | ###### | 6 | 560038 |
| SG | ###### | 6 | 560123 |
| AU | #### | 4 | 3000 |
| NZ | #### | 4 | 6011 |
| TH | ##### | 5 | 10110 |
| ID | ##### | 5 | 12190 |
| VN | ###### | 6 | 700000 |
| PH | #### | 4 | 1200 |
| MY | ##### | 5 | 50200 |
| TW | ### or ###-## | 3 or 5 | 100 or 100-86 |

### Gaps and Concerns

1. **Encoding challenges:** JP (Shift_JIS), KR (EUC-KR), CN/TW (GB2312/Big5) data sources may need encoding conversion. GeoNames normalizes to UTF-8 which simplifies this.

2. **No official bulk downloads:** CN, TH, ID, VN, PH do not provide official bulk postal code downloads. GeoNames is the primary fallback for these countries.

3. **Address format complexity:** IN, ID, and VN have the most variable address formats due to regional differences. SG, AU, NZ have the most standardized formats.

4. **Name generation complexity:** SG and MY require ethnicity-aware name generation due to mixed naming conventions. ID has mononyms. IN has extreme regional variation.

5. **Rapidly changing data:** ID regularly creates new provinces. VN reformed postal codes in 2017. CN periodically adjusts administrative boundaries. Data freshness should be monitored.

6. **Licensing:** Australia Post data has commercial use restrictions. Most other sources (GeoNames, government open data portals) are permissively licensed.

7. **Rural coverage gaps:** GeoNames data tends to be more complete for urban areas across all countries. Rural postal code coverage may be sparse for CN, ID, VN, and PH.
