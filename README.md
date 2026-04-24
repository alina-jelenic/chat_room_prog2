# Projektna naloga pri predmetu Programiranje 2

## Vsebina projekta

V tem projektu bova ustvarila chat-room v realnem času. Z orodji, ki jih ponuja Rust, bova lahko hkrati vodila pogovore z več uporabniki z maksimalno stabilnostjo in varnostjo.

## Zakaj sva se odločila za chat-room?

Ta projekt sva izbrala iz več razlogov, saj sva po analizi več možnosti ugotovila, da je prilagojen delu v Rustu, tudi za nove uporabnike, in da bo na najboljši način prispeval k razvoju naših programskih veščin v tem jeziku. Bolj natančno, delo na tem projektu nam bo pomagalo napredovati natanko na tistih področjih, ki jih potrebujemo.

Z uporabo asinhronega modela bova ustvarila sistem, ki temelji na arhitekturi odjemalec–strežnik, kjer se več odjemalcev poveže na strežnik, ta pa skrbi za posredovanje sporočil med njimi.

## Struktura projekta

├── Cargo.toml # konfiguracija projekta in odvisnosti
├── README.md # opis projekta
└── src/
    ├── main.rs # vstopna točka programa (zagon aplikacije)
    │
    ├── server/ # strežniški del aplikacije
    │   ├── mod.rs # definicija modula server
    │   └── connection.rs # logika posamezne povezave z odjemalcem
    │
    ├── client/ # odjemalski del aplikacije
    │ └── mod.rs # definicija modula client
    │
    └── common/ # skupni podatkovni tipi
    |    └── mod.rs # npr. struktura Message.
