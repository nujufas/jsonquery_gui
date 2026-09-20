# Play Console answers for jsonquery

What to enter for the declarations under **Policy and programs → App content**
(and the store listing) — accurate for the app as built. Re-check them if the
app gains network features, analytics, ads or new permissions.

## Store listing

- Category: **Tools** (alternative: Productivity). Tags: developer tools.
- Contact email: yours (public). Website: `https://github.com/nujufas/jsonquery_gui`.
- Text and graphics: `../play/metadata/android/en-US/`.

## App access

All functionality is available without login. *"All functionality is available
without special access."*

## Ads

**No, my app does not contain ads.**

## Content rating (IARC questionnaire)

Category **Utility, Productivity, Communication, or Other**. Answer **No** to
every question (violence, sexuality, language, controlled substances,
gambling, user-generated content shared with other users, location sharing,
purchases). Expected rating: **Everyone / PEGI 3 / USK 0**.

## Target audience and content

Target age groups: **18 and over** (a developer tool; not designed for
children, so it is not a "Families" app). No appeal to children.

## Data safety

- Does your app collect or share any of the required user data types? **No.**
- Is all of the user data collected by your app encrypted in transit? *(Not
  applicable — nothing is collected.)*
- Data deletion: not applicable.

Rationale, for reviewers: documents and queries are processed on-device only;
there are no accounts, analytics, ads or crash-reporting SDKs. The `INTERNET`
permission is used only to download a document from a URL the user types
("Open URL"), directly from that server to the device — nothing about the user
is sent anywhere, so no data is "collected" or "shared" in Play's sense.

## Permissions declarations

Only `android.permission.INTERNET` (normal permission; no declaration form).
No storage, contacts, location, camera or microphone access: files are opened
and saved through the system document picker.

## Government apps / financial features / health / news

None of them: **No** / not applicable.

## Advertising ID

The app does **not** use the advertising ID.
