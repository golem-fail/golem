# Geo Data Research: Americas

Research findings for generating fake geographic data for Americas countries.
Covers postal codes, administrative divisions, address formats, phone formats, and name ordering.

---

## Table of Contents

1. [United States (US)](#united-states-us)
2. [Canada (CA)](#canada-ca)
3. [Mexico (MX)](#mexico-mx)
4. [Brazil (BR)](#brazil-br)
5. [Argentina (AR)](#argentina-ar)
6. [Colombia (CO)](#colombia-co)
7. [Chile (CL)](#chile-cl)
8. [Peru (PE)](#peru-pe)

---

## United States (US)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **GeoNames** | https://download.geonames.org/export/zip/US.zip | TSV | ~43,000 ZIP codes with lat/long, state, county, city. CC BY 4.0. Excellent coverage. Updated regularly. |
| **US Census Bureau ZCTA** | https://www.census.gov/geographies/reference-files/time-series/geo/gazetteer-files.html | TSV | ZIP Code Tabulation Areas with coordinates and land/water area. Free public domain data. |
| **HUD USPS ZIP Code Crosswalk** | https://www.huduser.gov/portal/datasets/usps_crosswalk.html | Excel/CSV | Quarterly mapping of ZIP codes to census tracts, counties, and CBSAs. Free after registration. |
| USPS ZIP Code Lookup | https://tools.usps.com/zip-code-lookup.htm | API/Web | Official USPS lookup tool. Not bulk-downloadable. Requires API access for programmatic use. |
| OpenDataSoft | https://public.opendatasoft.com/explore/dataset/us-zip-code-latitude-and-longitude/ | CSV/JSON/API | ~43,000 entries with coordinates, city, state, county, timezone. |
| SimpleMaps | https://simplemaps.com/data/us-zips | CSV | Free basic tier (~33,000 ZIPs). Includes coordinates, population, density, county. |

**Recommended primary source:** GeoNames for comprehensive ZIP-to-place mapping with coordinates. Census Bureau ZCTA for authoritative geographic boundaries.

### Administrative Divisions (Cities/States)

- **50 states** + District of Columbia + 5 territories (Puerto Rico, Guam, US Virgin Islands, American Samoa, Northern Mariana Islands)
- **3,143 counties** and county-equivalents (parishes in Louisiana, boroughs/census areas in Alaska, independent cities in Virginia)
- **~19,500 incorporated places** (cities, towns, villages, boroughs)
- Source: US Census Bureau FIPS codes: https://www.census.gov/library/reference/code-lists/ansi.html
- Wikipedia: https://en.wikipedia.org/wiki/U.S._state
- State abbreviations: USPS 2-letter codes (e.g., CA, NY, TX, FL)

### Address Format

```
[Street number] [Street name] [Street suffix]
[Apt/Suite/Unit (optional)]
[City], [State abbreviation] [ZIP code]

Example:
123 Main Street
Apt 4B
Springfield, IL 62704
```

- **Postal code format:** `#####` or `#####-####` (ZIP+4). 5-digit base code, optional 4-digit extension.
  - Range: 00501 (IRS, Holtsville NY) to 99950 (Ketchikan AK)
  - First digit: broad region (0=NE, 9=AK/HI/West Coast)
  - Regex (basic): `^\d{5}(-\d{4})?$`
- **Street number:** Before street name. Typically 1-99999. Can include fractions (123 1/2) or letters.
- **Street suffixes:** Street (St), Avenue (Ave), Boulevard (Blvd), Drive (Dr), Lane (Ln), Road (Rd), Court (Ct), Place (Pl), Way, Circle (Cir), Terrace (Ter), Trail (Trl), Parkway (Pkwy)
- **Directionals:** N, S, E, W, NE, NW, SE, SW (before or after street name)

### Phone Number Format

- **Country code:** +1
- **Format patterns:**
  - General: `+1 (###) ###-####` or `+1-###-###-####`
  - All numbers are 10 digits: 3-digit area code + 7-digit local number
  - Area codes: cannot start with 0 or 1; second digit was historically 0 or 1 (pre-1995)
  - Exchange (next 3 digits): cannot start with 0 or 1; cannot be N11 (e.g., 911, 411)
  - Mobile and landline share the same format (no prefix distinction)
- **Domestic format:** `(###) ###-####`

### Name Ordering

- **given_first** (e.g., John Smith)

### Data Quality Assessment

- **Excellent.** The US has extensive, freely available geographic data. GeoNames provides the best single-file download of ZIP codes with coordinates. The Census Bureau offers authoritative boundary files. The ZIP code format is simple and well-understood. Main complexity is the large number of entries (~43,000 ZIPs) and occasional ZIP codes that span state lines or belong to unique entities (PO boxes, military).

---

## Canada (CA)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **GeoNames** | https://download.geonames.org/export/zip/CA.zip | TSV | ~870,000 entries (very granular — postal codes are 6-char). Includes province, place name, coordinates. CC BY 4.0. |
| **Statistics Canada** | https://www150.statcan.gc.ca/n1/en/catalogue/92-179-X | Various | Postal Code Conversion File (PCCF). Maps postal codes to census geography. Requires licence/purchase. |
| **Canada Post** | https://www.canadapost-postescanada.ca/cpc/en/support/kb/addressing/postal-code/postal-code-look-up.page | Web/API | Official lookup. Not freely bulk-downloadable. Commercial licence required for full dataset. |
| OpenDataSoft | https://public.opendatasoft.com/explore/dataset/georef-canada-province/ | CSV/JSON/API | Province and territory reference data with geometries. |
| CanadaCities (GitHub) | https://github.com/shivam-maharshi/canada-cities | CSV | Community dataset of Canadian cities with coordinates, province. |

**Recommended primary source:** GeoNames for the most comprehensive freely available postal code dataset. Forward Sortation Area (first 3 characters) lists are also available for lighter-weight usage.

### Administrative Divisions (Cities/States)

- **13 provinces and territories:** 10 provinces (Ontario, Quebec, British Columbia, Alberta, Manitoba, Saskatchewan, Nova Scotia, New Brunswick, Newfoundland and Labrador, Prince Edward Island) + 3 territories (Yukon, Northwest Territories, Nunavut)
- **~5,000 census subdivisions** (municipalities: cities, towns, villages, townships, etc.)
- Source: Statistics Canada geographic hierarchy: https://www12.statcan.gc.ca/census-recensement/2021/geo/ref/domain-domaine/index2021-eng.cfm
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_and_territories_of_Canada
- Province abbreviations: 2-letter codes (ON, QC, BC, AB, MB, SK, NS, NB, NL, PE, YT, NT, NU)

### Address Format

```
[Street number] [Street name] [Street type]
[Unit/Apt (optional)]
[City] [Province abbreviation]  [Postal code]

Example:
350 Albert Street
Suite 1400
Ottawa ON  K1R 1A4
```

- **Postal code format:** `A#A #A#` (letter-digit-letter space digit-letter-digit). Always uppercase.
  - First letter: Forward Sortation Area (FSA) — indicates province/region
  - Letters D, F, I, O, Q, U are never used; W and Z not used as first letter
  - Regex: `^[ABCEGHJ-NPRSTVXY]\d[ABCEGHJ-NPRSTV-Z]\s?\d[ABCEGHJ-NPRSTV-Z]\d$`
  - FSA first letter mapping: A=NL, B=NS, C=PE, E=NB, G/H/J=QC, K/L/M/N/P=ON, R=MB, S=SK, T=AB, V=BC, X=NT/NU, Y=YT
- **Street number:** Before street name. Typically 1-9999.
- **Street types:** Street (St), Avenue (Ave), Boulevard (Blvd), Drive (Dr), Road (Rd), Crescent (Cres), Way, Place (Pl), Court (Ct), Lane, Trail
- **Bilingual:** French-language addresses in Quebec follow similar structure but with French terms (Rue, Avenue, Boulevard, Chemin)

### Phone Number Format

- **Country code:** +1 (shared with US — NANP)
- **Format patterns:**
  - Same as US: `+1 (###) ###-####`
  - 10 digits: 3-digit area code + 7-digit local number
  - Canadian area codes: 204/431 (MB), 226/249/289/343/365/416/437/519/548/613/647/705/807/905 (ON), 236/250/604/672/778 (BC), 306/639 (SK), 403/587/780/825 (AB), 418/438/450/514/579/581/819/873 (QC), 428/506 (NB), 709 (NL), 782/902 (NS/PE), 867 (YT/NT/NU)
- **Domestic format:** `(###) ###-####`

### Name Ordering

- **given_first** (e.g., Sarah Johnson)

### Data Quality Assessment

- **Good.** GeoNames provides excellent coverage of Canadian postal codes (~870K entries). The postal code format is more complex than US ZIP codes (alternating letter-digit pattern with specific letter restrictions), requiring careful regex validation. The FSA-to-province mapping enables geographic validation. Main limitation is that Canada Post's official data is commercially licensed, so GeoNames is the best free alternative.

---

## Mexico (MX)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **SEPOMEX (Correos de Mexico)** | https://www.correosdemexico.gob.mx/SSLServicios/ConsultaCP/CodigoPostal_Exportar.aspx | TXT/XML | Official Mexican postal service. ~145,000 entries. Free download. Includes state, municipality, city, settlement (colonia), and settlement type. |
| **GeoNames** | https://download.geonames.org/export/zip/MX.zip | TSV | ~145,000 entries. Romanized names. CC BY 4.0. |
| **datos.gob.mx** | https://datos.gob.mx/busca/dataset/catalogo-nacional-de-codigos-postales | CSV/JSON | Official government open data portal. Same SEPOMEX data in machine-readable formats. |

**Recommended primary source:** SEPOMEX official dataset via datos.gob.mx for the most comprehensive and authoritative data. GeoNames as a convenient alternative with standardized format.

### Administrative Divisions (Cities/States)

- **32 federal entities (entidades federativas):** 31 states + 1 federal district (Ciudad de Mexico, formerly Distrito Federal)
- States: Aguascalientes, Baja California, Baja California Sur, Campeche, Chiapas, Chihuahua, Coahuila, Colima, Durango, Guanajuato, Guerrero, Hidalgo, Jalisco, Mexico (Estado de Mexico), Michoacan, Morelos, Nayarit, Nuevo Leon, Oaxaca, Puebla, Queretaro, Quintana Roo, San Luis Potosi, Sinaloa, Sonora, Tabasco, Tamaulipas, Tlaxcala, Veracruz, Yucatan, Zacatecas, Ciudad de Mexico
- **2,469 municipalities (municipios)** + 16 alcaldias (boroughs) in CDMX
- Source: INEGI (Instituto Nacional de Estadistica y Geografia): https://www.inegi.org.mx/
- Wikipedia: https://en.wikipedia.org/wiki/States_of_Mexico

### Address Format

```
[Street name] [Street number] [Interior number (optional)]
[Colonia (neighborhood)]
[Postal code] [City/Locality], [State]

Example:
Avenida Reforma 222 Int. 5
Colonia Juarez
06600 Ciudad de Mexico, CDMX
```

- **Postal code format:** `#####` (5 digits)
  - Range: 01000-99999
  - First two digits correspond to state (e.g., 01-16 = CDMX, 20 = Aguascalientes, 44-49 = Jalisco)
  - Regex: `^\d{5}$`
- **Street number:** After street name (opposite to US/CA convention). Typically 1-9999. "S/N" (sin numero) for unnumbered.
- **Interior number:** "Int." prefix for apartment/suite numbers
- **Colonia:** Neighborhood/settlement — a key part of Mexican addresses, essential for mail delivery
- **Street types:** Calle, Avenida (Av.), Boulevard (Blvd.), Calzada (Calz.), Cerrada (Cda.), Privada (Priv.), Periferico, Circuito

### Phone Number Format

- **Country code:** +52
- **Format patterns:**
  - All numbers: `+52 ## #### ####` (10 digits total)
  - Mexico unified to 10-digit dialing in 2019 (eliminated separate mobile prefix "1")
  - Major cities: CDMX (55), Guadalajara (33), Monterrey (81) use 2-digit area codes + 8-digit local
  - Other areas: 3-digit area code + 7-digit local
  - Mobile and landline share the same format (no prefix distinction since 2019)
- **Domestic format:** `## #### ####` (10 digits)

### Name Ordering

- **given_first** (e.g., Carlos Garcia Lopez)
- Typically two surnames: paternal (apellido paterno) + maternal (apellido materno)
- Full format: [Given name(s)] [Paternal surname] [Maternal surname]

### Data Quality Assessment

- **Very good.** SEPOMEX provides a comprehensive, free, official dataset with detailed colonia-level granularity. The unique feature of Mexican addresses is the "colonia" (neighborhood), which is essential for proper addressing. The postal code system is straightforward (5 digits). GeoNames coverage is excellent. Main considerations are handling accented characters (e.g., Queretaro, Mexico, Yucatan) and the colonia field.

---

## Brazil (BR)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Correios (official)** | https://www2.correios.com.br/sistemas/buscacep/ | Web/API | Official Brazilian postal service. CEP (Codigo de Enderecamento Postal) lookup. Not freely bulk-downloadable. ~1 million CEPs. |
| **GeoNames** | https://download.geonames.org/export/zip/BR.zip | TSV | ~560,000 entries. Good coverage. CC BY 4.0. |
| **Brasil API (community)** | https://brasilapi.com.br/api/cep/v2/{cep} | REST API | Free, community-maintained API for CEP lookup. Open source. |
| **CEP Aberto** | https://cepaberto.com/ | REST API/CSV | Community project. Free tier with API access. ~1 million CEPs with coordinates. |
| **ViaCEP** | https://viacep.com.br/ | REST API (JSON/XML) | Free CEP lookup API. Very popular. Returns street, neighborhood, city, state. |
| **basedosdados** | https://basedosdados.org/ | BigQuery/CSV | Brazilian open data platform. Includes geographic reference tables. |

**Recommended primary source:** GeoNames for bulk download. ViaCEP or Brasil API for programmatic lookup. CEP Aberto for comprehensive dataset with coordinates.

### Administrative Divisions (Cities/States)

- **26 states + 1 federal district** (Distrito Federal / Brasilia)
- States: Acre (AC), Alagoas (AL), Amapa (AP), Amazonas (AM), Bahia (BA), Ceara (CE), Espirito Santo (ES), Goias (GO), Maranhao (MA), Mato Grosso (MT), Mato Grosso do Sul (MS), Minas Gerais (MG), Para (PA), Paraiba (PB), Parana (PR), Pernambuco (PE), Piaui (PI), Rio de Janeiro (RJ), Rio Grande do Norte (RN), Rio Grande do Sul (RS), Rondonia (RO), Roraima (RR), Santa Catarina (SC), Sao Paulo (SP), Sergipe (SE), Tocantins (TO), Distrito Federal (DF)
- **5,570 municipalities (municipios)**
- 5 macro-regions: Norte, Nordeste, Centro-Oeste, Sudeste, Sul
- Source: IBGE (Instituto Brasileiro de Geografia e Estatistica): https://www.ibge.gov.br/
- Municipality codes: IBGE 7-digit code system
- Wikipedia: https://en.wikipedia.org/wiki/States_of_Brazil

### Address Format

```
[Street type] [Street name], [Number]
[Complement (optional)] - [Bairro (neighborhood)]
[CEP] [City] - [State abbreviation]

Example:
Rua Augusta, 1234
Apto 56 - Consolacao
01305-100 Sao Paulo - SP
```

- **Postal code (CEP) format:** `#####-###` (5 digits, hyphen, 3 digits)
  - First digit: macro-region (0-1=SP metro, 2=RJ/ES, 3=MG, 4=BA/SE, 5=NE, 6=N, 7=CO/DF, 8=PR/SC, 9=RS)
  - Regex: `^\d{5}-?\d{3}$`
- **Street number:** After street name, separated by comma. Typically 1-9999. "S/N" (sem numero) for unnumbered.
- **Bairro:** Neighborhood — important part of Brazilian addresses
- **Street types:** Rua (R.), Avenida (Av.), Alameda (Al.), Travessa (Tv.), Praca (Pc.), Estrada (Est.), Rodovia (Rod.), Largo

### Phone Number Format

- **Country code:** +55
- **Format patterns:**
  - Landline: `+55 ## ####-####` (2-digit area code + 8-digit local)
  - Mobile: `+55 ## 9####-####` (2-digit area code + 9 + 8 digits; mobile numbers start with 9 since 2014)
  - Area codes (DDD): 2 digits, range 11-99 (e.g., 11=Sao Paulo, 21=Rio, 31=Belo Horizonte, 61=Brasilia, 51=Porto Alegre)
  - Total digits: 10 (landline) or 11 (mobile), excluding country code
- **Domestic format:** `(##) ####-####` (landline) or `(##) 9####-####` (mobile)

### Name Ordering

- **given_first** (e.g., Maria Silva)

### Data Quality Assessment

- **Good.** Brazil has a complex CEP system with over 1 million codes. Correios does not offer a free bulk download, making community sources (ViaCEP, CEP Aberto, Brasil API) essential. GeoNames provides solid coverage (~560K entries). The 8-digit CEP format is straightforward. Main considerations: mobile numbers have an extra digit (9 prefix), Portuguese accented characters (Sao Paulo, Ceara), and the bairro (neighborhood) field is significant for addresses.

---

## Argentina (AR)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **GeoNames** | https://download.geonames.org/export/zip/AR.zip | TSV | ~4,800 entries (CPA codes). CC BY 4.0. Includes province, place name, coordinates. |
| **Correo Argentino** | https://www.correoargentino.com.ar/formularios/cpa | Web | Official postal service. CPA (Codigo Postal Argentino) lookup. Not bulk-downloadable. |
| **datos.gob.ar** | https://datos.gob.ar/ | Various | Argentine government open data portal. Geographic datasets available. |
| **World Postal Codes** | https://worldpostalcode.com/argentina/ | Web | Community reference. Lists postal codes by province. |

**Recommended primary source:** GeoNames for the most accessible bulk dataset. Argentina's postal code system transitioned from 4-digit to CPA (letter + 4 digits + 3 letters) in 1998, but the 4-digit system remains widely used in practice.

### Administrative Divisions (Cities/States)

- **23 provinces + 1 autonomous city** (Ciudad Autonoma de Buenos Aires / CABA)
- Provinces: Buenos Aires, Catamarca, Chaco, Chubut, Cordoba, Corrientes, Entre Rios, Formosa, Jujuy, La Pampa, La Rioja, Mendoza, Misiones, Neuquen, Rio Negro, Salta, San Juan, San Luis, Santa Cruz, Santa Fe, Santiago del Estero, Tierra del Fuego, Tucuman
- **~2,200 departamentos/partidos** (departments/districts)
- Source: INDEC (Instituto Nacional de Estadistica y Censos): https://www.indec.gob.ar/
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_Argentina

### Address Format

```
[Street name] [Number] [Floor/Apt (optional)]
[Postal code] [City]
[Province]

Example:
Avenida Corrientes 1234 Piso 5 Dto. B
C1043AAZ Buenos Aires
Buenos Aires
```

- **Postal code format (CPA):** `A####AAA` (1 letter + 4 digits + 3 letters)
  - First letter: province (C=CABA, B=Buenos Aires, X=Cordoba, S=Santa Fe, etc.)
  - Legacy 4-digit format still widely used (e.g., 1043 for parts of Buenos Aires)
  - Regex (CPA): `^[A-Z]\d{4}[A-Z]{3}$`
  - Regex (legacy): `^\d{4}$`
- **Street number:** After street name. Typically 1-9999.
- **Floor/Apartment:** "Piso" (floor) + "Dto." or "Depto." (apartment)
- **Street types:** Calle, Avenida (Av.), Boulevard (Bv.), Pasaje (Psje.), Diagonal (Diag.)

### Phone Number Format

- **Country code:** +54
- **Format patterns:**
  - Landline: `+54 ## ####-####` (Buenos Aires area code 11; other cities 2-4 digit area codes)
  - Mobile: `+54 9 ## ####-####` (9 inserted after country code for international calls)
  - Total digits: 10 (area code + local number), excluding country code
  - Area codes: 11 (Buenos Aires), 351 (Cordoba), 341 (Rosario), 261 (Mendoza), 381 (Tucuman)
  - Larger cities: 2-digit area code + 8-digit local. Smaller cities: 3-4 digit area code + 6-7 digit local.
- **Domestic format:** `(0##) ####-####` (landline) or `(0##) 15-####-####` (mobile, prefix 15)

### Name Ordering

- **given_first** (e.g., Juan Perez)

### Data Quality Assessment

- **Fair.** Argentina's dual postal code system (legacy 4-digit vs. CPA 8-character) creates complexity. The full CPA system is not widely adopted in everyday use, and bulk datasets are limited. GeoNames provides ~4,800 entries which covers the main codes. The phone number system is somewhat complex with variable-length area codes and the mobile "9" prefix for international dialing. Government open data (datos.gob.ar) is improving but geographic datasets are less comprehensive than other countries in this list.

---

## Colombia (CO)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **4-72 (Servicios Postales Nacionales)** | https://www.4-72.com.co/ | Web | Official Colombian postal service. Codigo postal lookup. Not bulk-downloadable. |
| **GeoNames** | https://download.geonames.org/export/zip/CO.zip | TSV | ~5,700 entries. Includes department, municipality, coordinates. CC BY 4.0. |
| **datos.gov.co** | https://www.datos.gov.co/ | CSV/JSON/API | Colombian government open data portal. Postal code datasets available. |
| **CPC (Codigo Postal de Colombia)** | https://visor.codigopostal.gov.co/472/visor/ | Web map | Official postal code map viewer from the Ministry of ICT. |
| **DANE** | https://www.dane.gov.co/ | Various | National statistics agency. DIVIPOLA (political-administrative division) codes and geographic reference tables. |

**Recommended primary source:** GeoNames for bulk download. datos.gov.co for official government datasets. Colombia introduced its current 6-digit postal code system in 2010.

### Administrative Divisions (Cities/States)

- **32 departments (departamentos) + 1 capital district** (Bogota D.C.)
- Major departments: Antioquia, Atlantico, Bolivar, Boyaca, Caldas, Caqueta, Cauca, Cesar, Cordoba, Cundinamarca, Huila, Magdalena, Meta, Narino, Norte de Santander, Quindio, Risaralda, Santander, Sucre, Tolima, Valle del Cauca
- **1,122 municipalities (municipios)**
- Source: DANE DIVIPOLA: https://www.dane.gov.co/index.php/sistema-estadistico-nacional-sen/normas-y-estandares/nomenclaturas-y-clasificaciones/divipola
- Wikipedia: https://en.wikipedia.org/wiki/Departments_of_Colombia

### Address Format

```
[Street type] [Street number] # [Cross-street number] - [Building number]
[Neighborhood (optional)]
[City], [Department]
[Postal code]

Example:
Carrera 7 # 32 - 16
Bogota, Cundinamarca
110311
```

- **Postal code format:** `######` (6 digits)
  - First 2 digits: department (e.g., 11=Bogota, 05=Antioquia, 76=Valle del Cauca)
  - Next 2 digits: municipality/zone
  - Last 2 digits: delivery zone
  - Regex: `^\d{6}$`
- **Street numbering:** Uses a grid/coordinate system. Streets are numbered, not named (in most cities).
  - "Calle" = street (runs east-west), "Carrera" = avenue (runs north-south)
  - Format: `Carrera 7 # 32 - 16` means "Carrera 7, at the intersection with Calle 32, building 16"
- **Street types:** Calle (Cl.), Carrera (Cr. / Cra.), Diagonal (Dg.), Transversal (Tv.), Avenida (Av.), Circular (Cir.)

### Phone Number Format

- **Country code:** +57
- **Format patterns:**
  - Landline: `+57 # #######` (1-digit area code + 7-digit local; area codes: 1=Bogota, 2=SW, 4=NW, 5=N coast, 6=coffee region, 7=NE, 8=SE)
  - Mobile: `+57 3## #######` (prefix 3xx + 7 digits; operators: 300-302 Claro, 310-318 Tigo, 320-323 Movistar)
  - Total digits: 10 (mobile) or 8 (landline), excluding country code
  - Colombia is transitioning to unified 10-digit dialing
- **Domestic format:** `(#) #######` (landline) or `3## #######` (mobile)

### Name Ordering

- **given_first** (e.g., Andres Garcia Rodriguez)
- Two surnames common (paternal + maternal), same as other Hispanic countries

### Data Quality Assessment

- **Good.** Colombia's 6-digit postal code system (introduced 2010) is relatively modern and well-structured. GeoNames provides ~5,700 entries covering main areas. The coordinate-based street naming system (Calle/Carrera grid) is distinctive and systematic, making it easy to generate realistic addresses. The DANE DIVIPOLA system provides authoritative administrative division codes. Main consideration is that postal code adoption is still growing in rural areas.

---

## Chile (CL)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Correos de Chile** | https://www.correos.cl/web/guest/codigo-postal | Web | Official Chilean postal service. Lookup tool. Not bulk-downloadable. |
| **GeoNames** | https://download.geonames.org/export/zip/CL.zip | TSV | ~2,200 entries. Includes region, commune, coordinates. CC BY 4.0. |
| **IDE Chile (Infraestructura de Datos Geoespaciales)** | https://www.ide.cl/ | Various | National geospatial data infrastructure. Geographic reference data. |
| **datos.gob.cl** | https://datos.gob.cl/ | CSV/JSON/API | Chilean government open data portal. |
| **SUBDERE** | https://www.subdere.gov.cl/ | Various | Subsecretaria de Desarrollo Regional. Administrative division data. |

**Recommended primary source:** GeoNames for bulk download. Chile uses 7-digit postal codes introduced in the 2000s, replacing an older system.

### Administrative Divisions (Cities/States)

- **16 regions:** Arica y Parinacota, Tarapaca, Antofagasta, Atacama, Coquimbo, Valparaiso, Metropolitana de Santiago, O'Higgins, Maule, Nuble, Biobio, La Araucania, Los Rios, Los Lagos, Aysen, Magallanes y de la Antartica Chilena
- **56 provinces (provincias)**
- **346 communes (comunas)**
- Source: SUBDERE / Biblioteca del Congreso Nacional: https://www.bcn.cl/siit/mapas_vectoriales
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Chile

### Address Format

```
[Street name] [Number] [Apt/Office (optional)]
[Commune]
[City]
[Postal code]
[Region (optional)]

Example:
Avenida Libertador Bernardo O'Higgins 1449
Santiago Centro
Santiago
8340518
Region Metropolitana
```

- **Postal code format:** `#######` (7 digits)
  - First 2-3 digits indicate region/area
  - No standard separator
  - Regex: `^\d{7}$`
  - Some older references use 3-digit area codes (e.g., "Santiago = 750")
- **Street number:** After street name. Typically 1-9999. "S/N" for unnumbered.
- **Street types:** Calle, Avenida (Av.), Pasaje (Psje.), Paseo, Camino, Cerro, Diagonal
- **Notable:** The Avenida Libertador Bernardo O'Higgins (commonly "La Alameda") is the main avenue in Santiago

### Phone Number Format

- **Country code:** +56
- **Format patterns:**
  - Landline: `+56 ## ### ####` (2-digit area code + 7-digit local; Santiago=2, Valparaiso=32, Concepcion=41)
  - Mobile: `+56 9 #### ####` (prefix 9 + 8 digits)
  - Total digits: 9, excluding country code
- **Domestic format:** `(0##) ### ####` (landline) or `09 #### ####` (mobile)

### Name Ordering

- **given_first** (e.g., Pablo Gonzalez Munoz)
- Two surnames common (paternal + maternal)

### Data Quality Assessment

- **Fair.** Chile's 7-digit postal code system is less widely documented than other Latin American countries. GeoNames provides ~2,200 entries which is adequate for basic coverage. Official bulk data is not freely available from Correos de Chile. The administrative division structure is clear (region -> province -> commune). Main considerations: postal code adoption is variable in rural areas, and the 7-digit format documentation is sparse compared to countries like Mexico or Brazil.

---

## Peru (PE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **SERPOST** | https://www.serpost.com.pe/ | Web | Official Peruvian postal service. Limited online lookup. Not bulk-downloadable. |
| **GeoNames** | https://download.geonames.org/export/zip/PE.zip | TSV | ~900 entries. Includes department, province, coordinates. CC BY 4.0. |
| **datos.gob.pe** | https://www.datosabiertos.gob.pe/ | CSV/JSON | Peruvian government open data portal. Limited postal code datasets. |
| **INEI** | https://www.inei.gob.pe/ | Various | National statistics institute. UBIGEO (geographic location code) system — the primary geographic reference system used in Peru. |
| **World Postal Codes** | https://worldpostalcode.com/peru/ | Web | Community reference. Lists codes by department. |

**Recommended primary source:** GeoNames for basic coverage. INEI UBIGEO codes (6-digit geographic codes) are more widely used than postal codes in Peru's administrative systems. Peru's postal code system uses 5-digit codes (Lima 15001-15999) but is not universally adopted, especially outside Lima.

### Administrative Divisions (Cities/States)

- **25 regions (departamentos/regiones) + 1 constitutional province** (Callao) + Lima Province (treated separately)
- Regions: Amazonas, Ancash, Apurimac, Arequipa, Ayacucho, Cajamarca, Cusco, Huancavelica, Huanuco, Ica, Junin, La Libertad, Lambayeque, Lima (region), Loreto, Madre de Dios, Moquegua, Pasco, Piura, Puno, San Martin, Tacna, Tumbes, Ucayali
- **196 provinces (provincias)**
- **1,874 districts (distritos)**
- Source: INEI UBIGEO system: https://www.inei.gob.pe/
- UBIGEO format: 6 digits (2 department + 2 province + 2 district)
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Peru

### Address Format

```
[Street type] [Street name] [Number]
[Urbanizacion/neighborhood (optional)]
[District]
[Province] - [Department]
[Postal code (optional)]

Example:
Jiron de la Union 300
Cercado de Lima
Lima - Lima
Lima 15001
```

- **Postal code format:** `#####` (5 digits) — Lima area codes are most well-defined (15001-15999)
  - First 2 digits loosely correspond to department
  - Postal codes are not universally used in everyday addresses
  - Regex: `^\d{5}$`
  - UBIGEO codes (6 digits) are more commonly referenced in official systems
- **Street number:** After street name. Typically 1-9999. "S/N" for unnumbered.
- **Urbanizacion (Urb.):** Named subdivisions/neighborhoods — often included in addresses
- **Street types:** Jiron (Jr.), Avenida (Av.), Calle (Ca.), Pasaje (Psje.), Alameda, Malecon, Ovalo, Prolongacion (Prol.)
- **"Jiron"** is a distinctly Peruvian term for a main street, commonly used in Lima

### Phone Number Format

- **Country code:** +51
- **Format patterns:**
  - Landline (Lima): `+51 1 ### ####` (area code 1 + 7 digits)
  - Landline (other): `+51 ## ## ####` (2-digit area code + 6 digits)
  - Mobile: `+51 9## ### ###` (prefix 9 + 8 digits)
  - Total digits: 8-9 (landline) or 9 (mobile), excluding country code
  - Area codes: 1 (Lima/Callao), 41 (Arequipa area), 44 (Trujillo area), 54 (Arequipa city), 64 (Huancayo), 74 (Chiclayo), 84 (Cusco)
- **Domestic format:** `(01) ### ####` (Lima) or `(0##) ## ####` (provinces) or `9## ### ###` (mobile)

### Name Ordering

- **given_first** (e.g., Maria Fernandez Lopez)
- Two surnames common (paternal + maternal)

### Data Quality Assessment

- **Fair to limited.** Peru's postal code system is the least developed of the countries in this research set. Postal codes are not universally used, especially outside of Lima. The UBIGEO system (6-digit geographic codes) is more widely used in official/administrative contexts. GeoNames provides only ~900 entries, the smallest dataset in this group. SERPOST does not offer a bulk download. For fake data generation, using the UBIGEO-based district/province/department hierarchy may be more realistic than relying heavily on postal codes. The phone system recently standardized to 9-digit mobile numbers. "Jiron" as a street type is uniquely Peruvian.

---

## Summary Comparison

| Country | Postal Code Format | # Codes (GeoNames) | Phone Country Code | Phone Digits | Address Number Position | Best Free Source |
|---------|-------------------|--------------------|--------------------|-------------|------------------------|-----------------|
| US | `#####` or `#####-####` | ~43,000 | +1 | 10 | Before street | GeoNames |
| CA | `A#A #A#` | ~870,000 | +1 | 10 | Before street | GeoNames |
| MX | `#####` | ~145,000 | +52 | 10 | After street | SEPOMEX/GeoNames |
| BR | `#####-###` | ~560,000 | +55 | 10-11 | After street | GeoNames/ViaCEP |
| AR | `A####AAA` (CPA) / `####` | ~4,800 | +54 | 10 | After street | GeoNames |
| CO | `######` | ~5,700 | +57 | 8-10 | After street (grid system) | GeoNames |
| CL | `#######` | ~2,200 | +56 | 9 | After street | GeoNames |
| PE | `#####` | ~900 | +51 | 8-9 | After street | GeoNames |

### Key Regional Patterns

1. **Name ordering:** All 8 countries use **given_first** ordering.
2. **Two surnames:** All Spanish-speaking countries (MX, AR, CO, CL, PE) and Brazil commonly use two surnames (paternal + maternal). US and Canada typically use one surname.
3. **Street number position:** US and Canada place the number before the street name; all Latin American countries place it after.
4. **Neighborhood field:** Mexico (colonia), Brazil (bairro), Peru (urbanizacion), and Colombia (barrio) all have neighborhood/settlement fields that are significant for addressing.
5. **GeoNames coverage:** Excellent for US, CA, MX, BR. Adequate for CO, AR, CL. Limited for PE.
6. **Phone formats:** US and Canada share the NANP (+1). Latin American countries each have distinct formats, with mobile numbers generally distinguishable by prefix (9 in BR/CL/PE, 3xx in CO).
7. **Postal code complexity:** Canada's alternating letter-digit format is the most complex. Argentina's dual system (legacy 4-digit vs. CPA 8-character) adds implementation complexity. Peru's postal code system is the least mature.
