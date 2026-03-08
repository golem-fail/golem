# Geo Data Research: Africa & Middle East

Research findings for generating fake geographic data for Africa and Middle East countries.
Covers postal codes, administrative divisions, address formats, phone formats, and name ordering.

---

## Table of Contents

1. [South Africa (ZA)](#south-africa-za)
2. [Nigeria (NG)](#nigeria-ng)
3. [Kenya (KE)](#kenya-ke)
4. [Egypt (EG)](#egypt-eg)
5. [Morocco (MA)](#morocco-ma)
6. [United Arab Emirates (AE)](#united-arab-emirates-ae)
7. [Saudi Arabia (SA)](#saudi-arabia-sa)
8. [Israel (IL)](#israel-il)

---

## South Africa (ZA)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **South African Post Office** | https://www.postoffice.co.za/postalcodes | Online lookup | Official source. No bulk download. Postal codes are 4 digits. |
| **GeoNames** | https://download.geonames.org/export/zip/ZA.zip | TSV | ~3,500 entries. CC BY 4.0. Includes place name, province, latitude/longitude. Good coverage. |
| **DataFirst (UCT)** | https://www.datafirst.uct.ac.za/ | Various | Academic data repository with South African geographic datasets. Registration required. |
| OpenStreetMap / Nominatim | https://nominatim.openstreetmap.org/ | API/JSON | Community-sourced. Variable coverage outside major cities. |

**Recommended primary source:** GeoNames for bulk postal code data. South African Post Office for validation.

### Administrative Divisions (Cities/States)

- **9 provinces:** Eastern Cape, Free State, Gauteng, KwaZulu-Natal, Limpopo, Mpumalanga, North West, Northern Cape, Western Cape
- **8 metropolitan municipalities:** City of Johannesburg, City of Cape Town, City of Tshwane (Pretoria), eThekwini (Durban), Ekurhuleni (East Rand), Nelson Mandela Bay (Port Elizabeth), Buffalo City (East London), Mangaung (Bloemfontein)
- **44 district municipalities**, **205 local municipalities**
- Major cities: Johannesburg, Cape Town, Durban, Pretoria, Port Elizabeth, Bloemfontein, East London, Polokwane, Nelspruit, Kimberley
- Source: Municipal Demarcation Board https://www.demarcation.org.za/
- Wikipedia: https://en.wikipedia.org/wiki/Provinces_of_South_Africa

### Address Format

```
[Building number] [Street name]
[Suburb]
[City]
[Province (optional)]
[Postal code]

Example:
12 Mandela Drive
Sandton
Johannesburg
Gauteng
2196
```

- **Postal code format:** `####` (4 digits). Range: 0001-9999. Generally organized by province.
  - 0001-0999: Gauteng (Pretoria area), Limpopo, North West
  - 1000-2199: Gauteng (Johannesburg area)
  - 2500-2999: Free State
  - 3000-3999: KwaZulu-Natal
  - 4000-4999: KwaZulu-Natal (Durban area)
  - 5000-5999: Eastern Cape
  - 6000-6999: Eastern Cape, Western Cape
  - 7000-7999: Western Cape (Cape Town area)
  - 8000-8999: Northern Cape
  - 9000-9999: Free State, Eastern Cape
  - Regex: `^\d{4}$`
- **Street number:** Before street name. Typically 1-999.
- **Street types:** Street, Road, Avenue, Drive, Lane, Close, Crescent, Place, Way (English names)
- **Suburb:** Important component; often more specific than city for delivery.

### Phone Number Format

- **Country code:** +27
- **Format patterns:**
  - Landline: `+27 ## ### ####` (area codes: 10/11=Johannesburg, 12=Pretoria, 21=Cape Town, 31=Durban, 41=Port Elizabeth, 51=Bloemfontein)
  - Mobile: `+27 6# ### ####`, `+27 7# ### ####`, `+27 8# ### ####` (prefixes: 060-069, 071-079, 081-084)
  - Total digits (excluding country code): 9
- **Domestic format:** `0## ### ####` (10 digits with leading 0)

### Name Ordering

- **given_first** (e.g., Thabo Mbeki, Nelson Mandela)
- Multilingual: English, Afrikaans, Zulu, Xhosa, Sotho, and 7 other official languages. Names reflect diverse ethnic backgrounds.

### Data Quality Assessment

- **Good.** GeoNames provides reasonable coverage of South African postal codes. The 4-digit postal code system is simple and regex-validatable. Main complexity is the importance of suburbs in addressing, and the multicultural naming conventions spanning 11 official languages. The South African Post Office does not offer a public bulk download, making GeoNames the practical choice.

---

## Nigeria (NG)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **NIPOST (Nigerian Postal Service)** | https://nipost.gov.ng/ | Online lookup | Official source. 6-digit postal codes introduced in 2012. No public bulk download available. |
| **GeoNames** | https://download.geonames.org/export/zip/NG.zip | TSV | Limited entries (~850). CC BY 4.0. Coverage is sparse outside major cities. |
| **Fake NIPOST datasets (community)** | Various GitHub repos | CSV/JSON | Community-assembled lists of varying quality. Cross-check against official NIPOST. |
| OpenStreetMap | https://www.openstreetmap.org/ | API | Some postal code boundaries mapped. Incomplete coverage. |

**Recommended primary source:** GeoNames for basic coverage. NIPOST website for validation. Note: postal code adoption is still partial in Nigeria; many addresses do not use them in practice.

### Administrative Divisions (Cities/States)

- **36 states + 1 Federal Capital Territory (FCT, Abuja)**
- States: Abia, Adamawa, Akwa Ibom, Anambra, Bauchi, Bayelsa, Benue, Borno, Cross River, Delta, Ebonyi, Edo, Ekiti, Enugu, Gombe, Imo, Jigawa, Kaduna, Kano, Katsina, Kebbi, Kogi, Kwara, Lagos, Nasarawa, Niger, Ogun, Ondo, Osun, Oyo, Plateau, Rivers, Sokoto, Taraba, Yobe, Zamfara, FCT Abuja
- **774 Local Government Areas (LGAs)**
- Major cities: Lagos, Abuja, Kano, Ibadan, Port Harcourt, Benin City, Kaduna, Enugu, Calabar, Warri, Abeokuta, Owerri
- Source: National Population Commission / Wikipedia: https://en.wikipedia.org/wiki/States_of_Nigeria

### Address Format

```
[Building number] [Street name]
[Area/District]
[City]
[State]
[Postal code (optional)]

Example:
24 Broad Street
Lagos Island
Lagos
Lagos State
101001
```

- **Postal code format:** `######` (6 digits). The first digit identifies the postal zone (1-9). Introduced in stages; not universally used.
  - Zone 1 (1xxxxx): Lagos
  - Zone 2 (2xxxxx): South West (Ogun, Oyo, Osun, Ondo, Ekiti)
  - Zone 3 (3xxxxx): South East / South-South
  - Zone 4 (4xxxxx): South East (Abia, Anambra, Enugu, Ebonyi, Imo)
  - Zone 5 (5xxxxx): South-South (Rivers, Bayelsa, Delta, Edo, Cross River, Akwa Ibom)
  - Zone 6 (6xxxxx): North Central (Benue, Kogi, Kwara, Nasarawa, Niger, Plateau)
  - Zone 7 (7xxxxx): North West (Kaduna, Katsina, Kano, Jigawa, Kebbi, Sokoto, Zamfara)
  - Zone 8 (8xxxxx): North East (Adamawa, Bauchi, Borno, Gombe, Taraba, Yobe)
  - Zone 9 (9xxxxx): FCT Abuja
  - Regex: `^\d{6}$`
- **Street number:** Before street name. Typically 1-999.
- **Street types:** Street, Road, Avenue, Close, Crescent, Way (English names, reflecting colonial heritage)
- **Area/District:** Important for navigation, especially in Lagos (e.g., Victoria Island, Ikeja, Lekki).

### Phone Number Format

- **Country code:** +234
- **Format patterns:**
  - Landline: `+234 # ### ####` (area codes: 1=Lagos, 2=Ibadan, 9=Abuja, 62=Benin, 64=Calabar, etc.)
  - Mobile: `+234 70# ### ####`, `+234 80# ### ####`, `+234 81# ### ####`, `+234 90# ### ####`, `+234 91# ### ####`
  - Mobile total digits (excluding country code): 10
  - Landline total digits (excluding country code): 7-8 (varies by area)
- **Domestic format:** `0### ### ####` (11 digits for mobile with leading 0)

### Name Ordering

- **given_first** (e.g., Chinua Achebe, Ngozi Okonjo-Iweala)
- Name patterns vary by ethnic group: Yoruba, Igbo, Hausa names have distinct patterns.
- Yoruba: often include day-of-birth names or family praise names (e.g., Oluwaseun, Adebayo).
- Igbo: often theophoric (e.g., Chukwuemeka, Nnamdi).
- Hausa: often Arabic-influenced (e.g., Muhammad, Ibrahim, Aisha).

### Data Quality Assessment

- **Fair.** Postal code data is sparse and adoption is incomplete. Many Nigerians do not use postal codes in daily addressing. The 6-digit system is relatively new and not comprehensively documented in open datasets. GeoNames coverage is limited. State/LGA data is well-documented. For fake data generation, using state + city + area/district is more realistic than relying on postal codes. Phone number patterns are well-documented.

---

## Kenya (KE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Posta Kenya (Postal Corporation of Kenya)** | https://www.posta.co.ke/ | Online lookup | Official source. 5-digit postal codes. No public bulk download. |
| **GeoNames** | https://download.geonames.org/export/zip/KE.zip | TSV | ~1,100 entries. CC BY 4.0. Covers major towns and cities. |
| **Kenya Open Data Initiative** | https://opendata.go.ke/ | Various | Government open data portal. Some geographic datasets available. |
| OpenStreetMap | https://www.openstreetmap.org/ | API | Community-mapped postal code data. Variable coverage. |

**Recommended primary source:** GeoNames for postal code mapping. Note: Kenyan addresses traditionally use P.O. Box numbers rather than street addresses, especially outside Nairobi.

### Administrative Divisions (Cities/States)

- **47 counties** (devolved government since 2010 Constitution):
  - Baringo, Bomet, Bungoma, Busia, Elgeyo-Marakwet, Embu, Garissa, Homa Bay, Isiolo, Kajiado, Kakamega, Kericho, Kiambu, Kilifi, Kirinyaga, Kisii, Kisumu, Kitui, Kwale, Laikipia, Lamu, Machakos, Makueni, Mandera, Marsabit, Meru, Migori, Mombasa, Murang'a, Nairobi, Nakuru, Nandi, Narok, Nyamira, Nyandarua, Nyeri, Samburu, Siaya, Taita-Taveta, Tana River, Tharaka-Nithi, Trans-Nzoia, Turkana, Uasin Gishu, Vihiga, Wajir, West Pokot
- **290 sub-counties**
- Major cities: Nairobi, Mombasa, Kisumu, Nakuru, Eldoret, Thika, Malindi, Nyeri, Machakos
- Source: Independent Electoral and Boundaries Commission (IEBC) / Wikipedia: https://en.wikipedia.org/wiki/Counties_of_Kenya

### Address Format

```
[Name]
P.O. Box [Number]-[Postal code]
[City/Town]

Example (P.O. Box style, traditional):
John Kamau
P.O. Box 12345-00100
Nairobi

Example (street address, increasingly common):
John Kamau
15 Kenyatta Avenue
Nairobi
00100
```

- **Postal code format:** `#####` (5 digits). Range: 00100-80400.
  - 00100-00999: Nairobi area
  - 01000-01999: Central Kenya (Kiambu, Thika, Nyeri area)
  - 10000-10999: Rift Valley
  - 20000-20999: Rift Valley (Nakuru, Kericho area)
  - 30000-30999: Western Kenya
  - 40000-40999: Nyanza (Kisumu area)
  - 50000-50999: Western Province
  - 60000-60999: Eastern (Embu, Meru)
  - 70000-70999: Coast hinterland
  - 80000-80999: Coast (Mombasa area)
  - Regex: `^\d{5}$`
- **P.O. Box addressing:** Traditional Kenyan addressing uses "P.O. Box [number]-[postal code]" format. Street addresses are increasingly used in urban areas but P.O. Box remains common.
- **Street types:** Road, Street, Avenue, Lane, Drive, Way (English names)
- **Street naming:** Mix of colonial-era English names and post-independence names (Kenyatta, Moi, Uhuru).

### Phone Number Format

- **Country code:** +254
- **Format patterns:**
  - Landline: `+254 20 ### ####` (Nairobi), `+254 41 ### ####` (Mombasa), `+254 ## ### ####` (other areas)
  - Mobile: `+254 7## ### ###` (prefixes: 700-729 Safaricom, 730-739 Airtel, 740-749, 750-759, 760-769, 770-779, 780-789, 790-799)
  - Mobile: `+254 1## ### ###` (newer allocations: 100-109, 110-119)
  - Total digits (excluding country code): 9
- **Domestic format:** `0### ### ###` (10 digits with leading 0)

### Name Ordering

- **given_first** (e.g., Wangari Maathai, Uhuru Kenyatta)
- Names reflect diverse ethnic groups: Kikuyu, Luo, Kalenjin, Kamba, Luhya, etc.
- Many Kenyans have an English first name + ethnic name + family name (e.g., James Mwangi Kamau).

### Data Quality Assessment

- **Fair.** GeoNames provides reasonable coverage of Kenyan postal codes. The main challenge is that traditional addressing uses P.O. Boxes rather than street addresses, making street-based fake address generation less realistic for many contexts. Street addressing is growing in Nairobi and Mombasa. The 47-county structure is well-documented. Phone number patterns are well-established, especially for mobile (Safaricom dominates with ~65% market share).

---

## Egypt (EG)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Egypt Post** | https://www.egyptpost.org/ | Online lookup | Official source. 5-digit postal codes. Limited public bulk data. |
| **GeoNames** | https://download.geonames.org/export/zip/EG.zip | TSV | ~3,200 entries. CC BY 4.0. Good coverage of governorates and major areas. |
| OpenStreetMap | https://www.openstreetmap.org/ | API | Some postal code data. Variable completeness. |

**Recommended primary source:** GeoNames for bulk data. Egypt Post for validation.

### Administrative Divisions (Cities/States)

- **27 governorates (muhafazat):**
  - Cairo, Giza, Alexandria, Qalyubia, Dakahlia, Sharqia, Gharbia, Monufia, Beheira, Kafr El Sheikh, Damietta, Port Said, Ismailia, Suez, North Sinai, South Sinai, Red Sea, Beni Suef, Fayoum, Minya, Asyut, Sohag, Qena, Luxor, Aswan, New Valley, Matruh
- Greater Cairo (Cairo + Giza + Qalyubia) is the dominant metropolitan area (~22 million).
- Major cities: Cairo, Alexandria, Giza, Shubra El-Kheima, Port Said, Suez, Luxor, Aswan, Tanta, Mansoura, Ismailia, Zagazig, Asyut
- Source: CAPMAS (Central Agency for Public Mobilization and Statistics) https://www.capmas.gov.eg/
- Wikipedia: https://en.wikipedia.org/wiki/Governorates_of_Egypt

### Address Format

```
[Building number] [Street name]
[District/Neighborhood]
[City]
[Governorate]
[Postal code]

Example (Latin script):
15 Tahrir Street
Downtown
Cairo
Cairo Governorate
11511

Example (Arabic script):
١٥ شارع التحرير
وسط البلد
القاهرة
محافظة القاهرة
١١٥١١
```

- **Postal code format:** `#####` (5 digits). Range: 11000-99999.
  - 11xxx: Cairo
  - 12xxx: Giza
  - 21xxx: Alexandria
  - 31xxx: Port Said
  - 41xxx: Ismailia
  - 42xxx: Suez
  - Regex: `^\d{5}$`
- **Bilingual addressing:** Addresses can be written in Arabic or Latin script. Arabic is official.
- **Street types (Arabic):** Sharia (شارع, street), Midan (ميدان, square), Haret (حارة, alley), Tariq (طريق, road/highway)
- **Street naming:** Mix of historical figures, dates, and Arabic descriptive names.
- **District/Neighborhood:** Important for navigation in large cities (e.g., Zamalek, Maadi, Heliopolis in Cairo).

### Phone Number Format

- **Country code:** +20
- **Format patterns:**
  - Landline: `+20 2 #### ####` (Cairo/Giza, 8-digit local), `+20 3 ### ####` (Alexandria, 7-digit local), `+20 ## ### ####` (other governorates)
  - Mobile: `+20 10 #### ####`, `+20 11 #### ####`, `+20 12 #### ####`, `+20 15 #### ####`
  - Mobile total digits (excluding country code): 10
  - Landline total digits (excluding country code): 8-9 (varies)
- **Domestic format:** `0## #### ####` (mobile), `02 #### ####` (Cairo landline)

### Name Ordering

- **given_first** (e.g., Mohamed Ahmed, Fatma Hassan)
- Arabic naming convention: [Given name] [Father's name] [Grandfather's name] [Family name]
- Common given names: Mohamed, Ahmed, Ali, Omar, Hassan, Ibrahim, Fatma, Mona, Nour, Amira
- Family names often derive from professions, places, or ancestors.

### Data Quality Assessment

- **Good.** GeoNames provides solid coverage of Egyptian postal codes. The 5-digit system is well-established. The 27 governorates are well-documented. Main complexity is bilingual addressing (Arabic/Latin) and the patronymic naming convention. Cairo's district/neighborhood system adds granularity. Phone formats are straightforward with 4 main mobile prefixes.

---

## Morocco (MA)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Barid Al-Maghrib (Morocco Post)** | https://www.poste.ma/ | Online lookup | Official postal service. 5-digit postal codes. No bulk download. |
| **GeoNames** | https://download.geonames.org/export/zip/MA.zip | TSV | ~1,500 entries. CC BY 4.0. Covers main cities and towns. |
| **data.gov.ma** | https://data.gov.ma/ | Various | Moroccan government open data portal. Some geographic datasets. |
| OpenStreetMap | https://www.openstreetmap.org/ | API | Community-mapped postal codes. Moderate coverage. |

**Recommended primary source:** GeoNames for bulk data. Barid Al-Maghrib for validation.

### Administrative Divisions (Cities/States)

- **12 regions:** Tanger-Tetouan-Al Hoceima, Oriental, Fes-Meknes, Rabat-Sale-Kenitra, Beni Mellal-Khenifra, Casablanca-Settat, Marrakech-Safi, Draa-Tafilalet, Souss-Massa, Guelmim-Oued Noun, Laayoune-Sakia El Hamra, Dakhla-Oued Ed-Dahab
- **75 provinces and prefectures**
- **1,503 communes**
- Major cities: Casablanca, Rabat, Fes, Marrakech, Tangier, Agadir, Meknes, Oujda, Kenitra, Tetouan, Sale, Nador
- Source: Haut-Commissariat au Plan https://www.hcp.ma/
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Morocco

### Address Format

```
[Building number] [Street name]
[District/Quartier]
[Postal code] [City]

Example (French style):
25 Boulevard Mohammed V
Quartier Hassan
10000 Rabat

Example (Arabic):
٢٥ شارع محمد الخامس
حي حسان
١٠٠٠٠ الرباط
```

- **Postal code format:** `#####` (5 digits). Range: 10000-99000.
  - 10000-19999: Rabat-Sale-Kenitra region
  - 20000-29999: Casablanca-Settat region
  - 30000-39999: Fes-Meknes region
  - 40000-49999: Marrakech-Safi region
  - 50000-59999: Beni Mellal-Khenifra / Draa-Tafilalet
  - 60000-69999: Souss-Massa / Guelmim-Oued Noun
  - 80000-89999: Oriental region
  - 90000-93999: Tanger-Tetouan-Al Hoceima region
  - Regex: `^\d{5}$`
- **Bilingual/trilingual:** Addresses commonly written in French and/or Arabic. Amazigh (Berber) also used in some regions.
- **Street types (French):** Rue, Avenue, Boulevard, Place, Passage, Impasse
- **Street types (Arabic):** Zanqa (زنقة, narrow street), Sharia (شارع, street), Derb (درب, alley in medina areas)
- **Postal code placement:** Before city name (French convention).

### Phone Number Format

- **Country code:** +212
- **Format patterns:**
  - Landline: `+212 5## ### ###` (prefixes: 522=Casablanca, 537=Rabat, 535=Fes, 524=Marrakech, 539=Tangier, 528=Agadir)
  - Mobile: `+212 6## ### ###` or `+212 7## ### ###`
  - Total digits (excluding country code): 9
- **Domestic format:** `0#-## ## ## ##` or `0### ### ###` (10 digits with leading 0)

### Name Ordering

- **given_first** (e.g., Mohammed Benali, Fatima Zahra El Fassi)
- Arabic naming conventions with Moroccan characteristics.
- Family names often begin with "El", "Ben", "Bou", or "Al" (e.g., El Amrani, Benkirane, Bouazza).
- Amazigh (Berber) names also common (e.g., Aziz Akhannouch).

### Data Quality Assessment

- **Good.** GeoNames covers ~1,500 Moroccan postal codes adequately. The 5-digit system is well-organized by region. Bilingual addressing (French/Arabic) is the main complexity. Morocco's administrative structure was reorganized in 2015 (12 regions replaced 16), so care is needed to use current divisions. The 12-region structure is now well-documented. Phone patterns are clean and well-defined.

---

## United Arab Emirates (AE)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Emirates Post** | https://www.emiratespost.ae/ | Online/Makani | UAE uses the Makani number system (10-digit geo-address) rather than traditional postal codes. Emirates Post provides P.O. Box delivery. |
| **GeoNames** | https://download.geonames.org/export/zip/AE.zip | TSV | Very limited entries. UAE historically has no traditional postal code system. |
| **Makani (Dubai Municipality)** | https://makani.ae/ | Online/App | Dubai's geo-addressing system assigns a 10-digit number to every building entrance. Not a postal code per se. |

**Recommended primary source:** The UAE does not have a traditional postal code system. Addressing relies on P.O. Box numbers, Makani numbers, or descriptive addresses. For fake data generation, using emirates + city + area/district is more realistic than postal codes.

### Administrative Divisions (Cities/States)

- **7 emirates:** Abu Dhabi, Dubai, Sharjah, Ajman, Umm Al Quwain, Ras Al Khaimah, Fujairah
- Abu Dhabi is the capital and largest emirate by area.
- Dubai is the largest by population and most commercially prominent.
- Major cities: Abu Dhabi, Dubai, Sharjah, Al Ain, Ajman, Ras Al Khaimah, Fujairah
- Key areas (Dubai): Deira, Bur Dubai, Jumeirah, Dubai Marina, Downtown Dubai, Business Bay, Al Barsha, Karama
- Key areas (Abu Dhabi): Khalifa City, Al Reem Island, Saadiyat Island, Al Ain, Yas Island
- Source: Federal Competitiveness and Statistics Authority / Wikipedia: https://en.wikipedia.org/wiki/Emirates_of_the_United_Arab_Emirates

### Address Format

```
[Name]
[Building name/number] [Street name]
[Area/District]
[City]
[Emirate]
P.O. Box [Number]

Example:
Ahmed Al Maktoum
Tower 5, Sheikh Zayed Road
Al Barsha
Dubai
UAE
P.O. Box 12345

Example (with Makani):
Makani: 1234567890
Dubai, UAE
```

- **No traditional postal codes.** The UAE relies on:
  - P.O. Box numbers (essential for mail delivery)
  - Makani numbers (10-digit geo-address for physical location)
  - Descriptive addressing (building name, street, area)
- **Building names:** Very common and important (e.g., Burj Khalifa, Emirates Towers, Etihad Towers).
- **Street naming:** Mix of numbered streets and named streets (e.g., Sheikh Zayed Road, Hamdan Street, Al Wasl Road).
- **Area/District:** Critical for navigation (e.g., Jumeirah, Deira, Khalidiya).

### Phone Number Format

- **Country code:** +971
- **Format patterns:**
  - Landline: `+971 2 ### ####` (Abu Dhabi), `+971 4 ### ####` (Dubai), `+971 6 ### ####` (Sharjah/Ajman/UAQ/Fujairah), `+971 7 ### ####` (RAK)
  - Mobile: `+971 5# ### ####` (prefixes: 50=Etisalat, 52=du, 54=Etisalat, 55=du, 56=du, 58=du)
  - Total digits (excluding country code): 8 (landline), 9 (mobile)
- **Domestic format:** `0# ### ####` (landline), `05# ### ####` (mobile)

### Name Ordering

- **given_first** (e.g., Mohammed bin Rashid Al Maktoum)
- Arabic naming convention: [Given name] bin/bint [Father's name] [Family/Tribal name]
- "bin" (son of) / "bint" (daughter of) used in formal names.
- "Al" prefix common in family names (e.g., Al Maktoum, Al Nahyan, Al Qassimi).
- Expatriate population (~85%) means names from South Asia, Southeast Asia, and other Arab countries are very common.

### Data Quality Assessment

- **Fair-to-Good (special case).** The UAE does not use traditional postal codes, making it unique among the countries in this study. For fake data generation, the key components are: emirate, city, area/district, building name, and P.O. Box number. The Makani system is well-documented for Dubai. The 7-emirate structure is simple and well-defined. Phone patterns are clean. The main challenge is the reliance on P.O. Boxes and descriptive addressing rather than structured postal codes.

---

## Saudi Arabia (SA)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Saudi Post (SPL)** | https://www.splonline.com.sa/ | Online lookup | Official source. 5-digit postal codes as part of the National Address system. |
| **National Address System (ASAN)** | https://na.sa/ | Online/API | Saudi Arabia's comprehensive address system launched ~2015. Assigns structured addresses to all buildings. |
| **GeoNames** | https://download.geonames.org/export/zip/SA.zip | TSV | ~1,200 entries. CC BY 4.0. Covers main cities and regions. |
| OpenStreetMap | https://www.openstreetmap.org/ | API | Growing coverage of Saudi addresses. |

**Recommended primary source:** GeoNames for bulk data. Saudi Post / National Address system for validation and structure reference.

### Administrative Divisions (Cities/States)

- **13 administrative regions (manatiq):**
  - Riyadh, Makkah, Madinah, Qassim, Eastern Province, Asir, Tabuk, Hail, Northern Borders, Jawf, Najran, Bahah, Jizan
- Each region has a capital city and multiple governorates.
- Major cities: Riyadh (capital), Jeddah, Makkah (Mecca), Madinah (Medina), Dammam, Dhahran, Al Khobar, Tabuk, Buraydah, Khamis Mushait, Taif, Abha, Hofuf
- Source: Saudi General Authority for Statistics https://www.stats.gov.sa/
- Wikipedia: https://en.wikipedia.org/wiki/Regions_of_Saudi_Arabia

### Address Format

```
[Building number] [Street name]
[District/Neighborhood]
[City]
[Postal code] [Additional code (4 digits)]

Example (National Address format):
Building 7892, King Fahd Road
Al Olaya District
Riyadh
12211-3456

Example (Arabic):
مبنى ٧٨٩٢ طريق الملك فهد
حي العليا
الرياض
١٢٢١١-٣٤٥٦
```

- **Postal code format:** `#####` or `#####-####` (5 digits, optionally followed by a 4-digit additional code).
  - The National Address system uses 5+4 format (like US ZIP+4).
  - Range: 11000-99999 (first 5 digits)
  - 11xxx-13xxx: Riyadh region
  - 21xxx-23xxx: Makkah region
  - 31xxx-35xxx: Eastern Province
  - 41xxx-42xxx: Madinah region
  - Regex (5 digit): `^\d{5}$`
  - Regex (5+4): `^\d{5}(-\d{4})?$`
- **Building number:** 4-digit number assigned by National Address system.
- **District (Hayy):** Important component in Saudi addresses.
- **Bilingual:** Arabic official; English commonly used in business.

### Phone Number Format

- **Country code:** +966
- **Format patterns:**
  - Landline: `+966 1# ### ####` (Riyadh area), `+966 2 ### ####` (Jeddah/Makkah), `+966 3 ### ####` (Eastern), `+966 4 ### ####` (North), `+966 6 ### ####` (South), `+966 7 ### ####` (South)
  - Mobile: `+966 5# ### ####` (prefixes: 50, 53, 54, 55, 56, 57, 58, 59)
  - Total digits (excluding country code): 9
- **Domestic format:** `0## ### ####` or `05# ### ####`

### Name Ordering

- **given_first** (e.g., Mohammed bin Salman Al Saud)
- Arabic naming convention: [Given name] bin/bint [Father's name] [Grandfather's name (optional)] [Family/Tribal name]
- "bin" (son of) / "bint" (daughter of) in formal contexts.
- "Al" prefix common in family/tribal names (e.g., Al Saud, Al Rashid, Al Dosari).
- Many names are Islamic/Arabic (e.g., Abdullah, Abdulrahman, Norah, Fatimah).

### Data Quality Assessment

- **Good.** Saudi Arabia has invested heavily in the National Address system (ASAN), which provides structured, comprehensive addressing. The 5+4 postal code system is well-designed. GeoNames provides reasonable coverage. The 13-region structure is stable and well-documented. The main complexity for fake data generation is the National Address format with its building numbers and additional codes. Phone patterns are straightforward.

---

## Israel (IL)

### Postal Code Database

| Source | URL | Format | Notes |
|--------|-----|--------|-------|
| **Israel Post** | https://www.israelpost.co.il/ | Online lookup | Official source. 7-digit postal codes (mitkod/mikud). Updated system since 2013. |
| **GeoNames** | https://download.geonames.org/export/zip/IL.zip | TSV | ~2,500 entries. CC BY 4.0. Good coverage including the 7-digit system. |
| **Israel CBS (Central Bureau of Statistics)** | https://www.cbs.gov.il/ | Various | Official statistics with geographic datasets. Locality codes and classifications. |
| OpenStreetMap | https://www.openstreetmap.org/ | API | Good coverage of Israeli addresses. |

**Recommended primary source:** GeoNames for bulk data. Israel Post for validation. CBS for locality/settlement data.

### Administrative Divisions (Cities/States)

- **6 administrative districts (mehozot):** Jerusalem, Northern, Haifa, Central, Tel Aviv, Southern
- **15 sub-districts (nafot)**
- Major cities: Jerusalem, Tel Aviv-Yafo, Haifa, Rishon LeZion, Petah Tikva, Ashdod, Netanya, Beer Sheva, Holon, Bnei Brak, Ramat Gan, Bat Yam, Ashkelon, Rehovot, Herzliya, Kfar Saba, Ra'anana, Modiin, Nazareth, Eilat
- Source: Israel CBS / Wikipedia: https://en.wikipedia.org/wiki/Districts_of_Israel

### Address Format

```
[Street name] [Building number]
[City]
[Postal code]

Example (Hebrew, right-to-left):
רחוב הרצל 15
תל אביב-יפו
6120101

Example (Latin script):
Herzl Street 15
Tel Aviv-Yafo
6120101
```

- **Postal code format:** `#######` (7 digits). Introduced in 2013, replacing the older 5-digit system.
  - Range: 1000000-9999999
  - First digit roughly corresponds to district:
    - 1xxxxxx-2xxxxxx: Jerusalem area
    - 3xxxxxx-4xxxxxx: Northern district / Haifa
    - 5xxxxxx-6xxxxxx: Central / Tel Aviv area
    - 7xxxxxx: Central / Tel Aviv
    - 8xxxxxx-9xxxxxx: Southern district
  - Regex: `^\d{7}$`
- **Street number placement:** After street name (unlike most Western countries).
- **Street types (Hebrew):** Rehov (רחוב, street), Sderot (שדרות, boulevard), Derech (דרך, road/way), Kikar (כיכר, square)
- **Bilingual:** Hebrew and Arabic are official languages. English widely used.

### Phone Number Format

- **Country code:** +972
- **Format patterns:**
  - Landline: `+972 2 ### ####` (Jerusalem), `+972 3 ### ####` (Tel Aviv area), `+972 4 ### ####` (Haifa/North), `+972 8 ### ####` (South), `+972 9 ### ####` (Sharon area)
  - Mobile: `+972 5# ### ####` (prefixes: 50, 51, 52, 53, 54, 55, 56, 58)
  - Total digits (excluding country code): 8 (landline), 9 (mobile)
- **Domestic format:** `0#-### ####` (landline), `05#-### ####` (mobile)

### Name Ordering

- **given_first** (e.g., David Ben-Gurion, Naftali Bennett)
- Hebrew names: Often biblical (e.g., David, Moshe, Sarah, Ruth) or modern Hebrew (e.g., Tal, Nir, Yael).
- Family names: Very diverse due to immigration from worldwide. Can be Hebrew, European, Arabic, or adapted.
- Compound surnames with "Ben-" (son of) are common (e.g., Ben-Gurion, Ben-Ami).

### Data Quality Assessment

- **Good.** GeoNames provides solid coverage of the 7-digit postal code system. The 2013 transition from 5-digit to 7-digit codes is well-documented. The 6-district structure is simple. Main complexities: right-to-left Hebrew text handling, street number after street name (reverse of Western convention), and the highly diverse naming conventions reflecting immigration from worldwide. Phone patterns are well-defined. The CBS provides excellent official geographic data.

---

## Cross-Country Summary

### Postal Code Systems Comparison

| Country | Code | Digits | Format | Open Data Quality | Notes |
|---------|------|--------|--------|-------------------|-------|
| South Africa | ZA | 4 | `####` | Good (GeoNames) | Simple numeric system |
| Nigeria | NG | 6 | `######` | Poor | Low adoption; NIPOST data not publicly available in bulk |
| Kenya | KE | 5 | `#####` | Fair (GeoNames) | P.O. Box addressing more common than street addressing |
| Egypt | EG | 5 | `#####` | Good (GeoNames) | Bilingual Arabic/English |
| Morocco | MA | 5 | `#####` | Good (GeoNames) | Bilingual French/Arabic |
| UAE | AE | N/A | Makani (10-digit) | N/A | No traditional postal codes; P.O. Box + Makani system |
| Saudi Arabia | SA | 5+4 | `#####-####` | Good (GeoNames) | National Address system with building numbers |
| Israel | IL | 7 | `#######` | Good (GeoNames) | Migrated from 5 to 7 digits in 2013 |

### Key Data Sources (All Countries)

| Source | URL | Coverage | License |
|--------|-----|----------|---------|
| **GeoNames Postal Codes** | https://download.geonames.org/export/zip/ | All 8 countries (varying quality) | CC BY 4.0 |
| **OpenStreetMap** | https://www.openstreetmap.org/ | Global | ODbL |
| **UN LOCODE** | https://unece.org/trade/cefact/UNLOCODE-Download | City/port codes for all countries | Free |
| **ISO 3166-2** | https://www.iso.org/iso-3166-country-codes.html | Subdivision codes for all countries | Via Wikipedia/Wikidata (free) |
| **Wikidata** | https://www.wikidata.org/ | Administrative divisions, city data | CC0 |

### Name Ordering Summary

All 8 countries use **given_first** name ordering. Arabic-speaking countries (EG, MA, AE, SA) commonly include patronymics (bin/bint) in formal names. Israel uses given_first with diverse surname origins.

### Addressing Challenges for Fake Data Generation

1. **Bilingual/multilingual addressing:** EG (Arabic/English), MA (French/Arabic), SA (Arabic/English), IL (Hebrew/Arabic/English), ZA (11 official languages)
2. **Non-postal-code addressing:** UAE (Makani + P.O. Box), KE (P.O. Box traditional)
3. **Low postal code adoption:** NG (6-digit codes not universally used)
4. **Cultural naming patterns:** Arabic patronymics, Kenyan multi-ethnic names, Nigerian ethnic-group-specific patterns, Israeli immigration-influenced diversity
5. **Script handling:** Arabic script (EG, MA, AE, SA), Hebrew script (IL)

### Phone Number Patterns Summary

| Country | Code | Mobile Pattern | Mobile Digits (excl. CC) |
|---------|------|---------------|--------------------------|
| ZA | +27 | 6x/7x/8x | 9 |
| NG | +234 | 70x/80x/81x/90x/91x | 10 |
| KE | +254 | 7xx/1xx | 9 |
| EG | +20 | 10/11/12/15 | 10 |
| MA | +212 | 6xx/7xx | 9 |
| AE | +971 | 5x | 9 |
| SA | +966 | 5x | 9 |
| IL | +972 | 5x | 9 |
