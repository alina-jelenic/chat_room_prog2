# Projektna naloga pri predmetu Programiranje 2

## Vsebina projekta

V tem projektu bova ustvarila chat-room v realnem času. Z orodji, ki jih ponuja Rust, bova lahko hkrati vodila pogovore z več uporabniki z maksimalno (ne)stabilnostjo in (ne)varnostjo.

## Zakaj sva se odločila za chat-room?

Ta projekt sva izbrala iz več razlogov, saj sva po analizi več možnosti ugotovila, da je prilagojen delu v Rustu, tudi za nove uporabnike, in da bo na najboljši način prispeval k razvoju naših programskih veščin v tem jeziku. Bolj natančno, delo na tem projektu nam bo pomagalo napredovati natanko na tistih področjih, ki jih potrebujemo.

Z uporabo asinhronega modela bova ustvarila sistem, ki temelji na arhitekturi odjemalec–strežnik, kjer se več odjemalcev poveže na strežnik, ta pa skrbi za posredovanje sporočil med njimi.

## Struktura projekta

```
├── Cargo.toml              # konfiguracija glavnega paketa in odvisnosti
├── README.md                # opis projekta
├── .env.example              # predloga za okoljske spremenljivke
├── src/
│   ├── main.rs                # vstopna točka: povezava z bazo, migracije, zagon strežnika
│   ├── lib.rs                 # javni moduli knjižnice (controller, entities)
│   ├── controller/
│   │   ├── mod.rs               # deklaracija podmodulov
│   │   ├── auth.rs              # JWT, seje, avtentikacijski middleware
│   │   ├── forms.rs             # obdelava prijave in registracije
│   │   ├── rooms.rs             # logika sob: ustvarjanje, pridružitev, sporočila
│   │   ├── tipi.rs              # deljeno stanje strežnika (SharedState)
│   │   └── web.rs               # axum router, WebSocket handler
│   └── entities/                # SeaORM modeli (client, soba, message, room_member)
├── migration/                 # ločen paket z migracijami (sea-orm-migration)
│   ├── src/
│   │   ├── main.rs               # CLI za poganjanje migracij
│   │   ├── lib.rs                 # seznam vseh migracij
│   │   └── m*.rs                   # posamezne migracijske datoteke
│   └── README.md                 # navodila za uporabo migracijskega CLI-ja
├── static/                    # statične HTML/CSS datoteke frontenda
│   ├── index.html                # glavni vmesnik klepetalnice
│   └── authorisation.html         # prijava in registracija
├── tests/
│   └── integration.rs            # integracijski testi (HTTP + WebSocket)
└── .github/workflows/ci.yml    # CI: fmt, clippy, testi
```

## Zagon projekta

## Glavne funkcionalnosti
 
- **Registracija in prijava** z uporabniškim imenom in geslom; gesla so
  zgoščena z argon2, seja pa se hrani v HttpOnly piškotku kot JWT žeton.
- **Klepetalne sobe**: soba `#general` je na voljo vsem, dodatne sobe pa
  lahko uporabniki ustvarijo (postanejo njihov lastnik) ali se jim
  pridružijo prek numeričnega ID-ja sobe.
- **Upravljanje članstva**: pridružitev, zapustitev sobe in brisanje sobe
  (samo lastnik), z ustreznimi omejitvami dostopa do zasebnih sob.
- **Sporočila v realnem času** prek WebSocket povezave in HTMX (`ws-send`,
  `hx-swap-oob`) brez ročnega pisanja JavaScripta na frontendu.
- **Zgodovina sporočil s straničenjem**: sporočila se nalagajo po straneh
  (50 na stran), starejša sporočila se naložijo na zahtevo.
- **Migracije baze**, ki ohranjajo obstoječe podatke pri nadgradnji sheme
  (npr. dodajanje stolpca za geslo ali povezovanje sporočil s sobami na
  starejših, že napolnjenih bazah).

