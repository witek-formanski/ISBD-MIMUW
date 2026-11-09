# Projekt 2 - kolumnowy format pliku

Program zapisuje tabelę do własnego pliku binarnego i odczytuje ją partiami. Podczas odczytu oblicza metryki dla każdej kolumny. Do projektu dołączono `sample.isbd` z jedną kolumną INT64, jedną BOOLEAN i dwiema VARCHAR.

## Uruchomienie

Potrzebne są Rust, Cargo i `make`. W katalogu `projects/proj2` należy wykonać:

```sh
make all
./target/release/proj2 sample.isbd
```

Wynik dla dołączonego pliku:

```text
Rows: 6
id (INT64): mean = 15.833333
active (BOOLEAN): binary entropy = 0.918296
name (VARCHAR): ASCII character count = 17
city (VARCHAR): ASCII character count = 33
```

Średnia obejmuje wszystkie wartości INT64, entropia binarna jest liczona z udziału wartości `true`, a licznik ASCII zlicza znaki o kodach 0–127. Inne znaki UTF-8 nie zwiększają licznika. Jeśli plik nie zawiera wierszy, średnia i entropia mają wynik `no data`.

Przykładowy plik można ponownie utworzyć poleceniem:

```sh
./target/release/generate_sample sample.isbd
```

Testy:

```sh
make test
```

## Układ pliku

Plik składa się z jednego nagłówka i kolejnych partii (batchy) wierszy. Nagłówek podaje kolejność, nazwy i typy kolumn. Każdy batch zawiera co najwyżej 1024 wiersze i osobny blok danych dla każdej kolumny. Odczyt kończy się po ostatnim pełnym batchu; w pliku nie ma osobnego znacznika końca ani łącznej liczby wierszy.

```text
MAGIC[4]  VERSION[u16 LE]  COLUMN_COUNT[VLE]
powtórz COLUMN_COUNT razy:
    TYPE[u8]  NAME_LENGTH[VLE]  NAME[NAME_LENGTH bajtów UTF-8]
powtarzaj do końca pliku:
    ROW_COUNT[u32 LE]
    powtórz COLUMN_COUNT razy, w kolejności z nagłówka:
        BLOCK_LENGTH[VLE]  BLOCK[BLOCK_LENGTH bajtów]
```

- `MAGIC` to cztery bajty ASCII: `ISBD`.
- `VERSION` ma wartość 1 (zapisaną w little endian).
- `TYPE` ma wartość `1` dla INT64, `2` dla BOOLEAN lub `3` dla VARCHAR.
- Długości nazw, bloków i napisów są liczbami bajtów. Nazwy kolumn muszą być niepuste i unikalne.
- `ROW_COUNT` dotyczy całego batcha. Każdy jego blok kolumnowy musi zawierać dokładnie tyle wartości.

### Kodowanie danych w blokach kolumnowych

**INT64.** W każdym batchu pierwsza wartość jest porównywana z zerem, a każda następna z poprzednią wartością i zapisywana jest różnica (w `i128`, aby uniknąć overflowa w `i64`). Następnie stosuje się kodowanie ZigZag i VLE.

**BOOLEAN.** Wartości są pakowane po osiem do jednego bajtu, zaczynając od najmłodszego bitu. Niewykorzystane bity ostatniego bajtu są zerowe. Każda wartość zajmuje w pliku dokładnie jeden bit.

**VARCHAR.** Najpierw tworzymy nieskompresowany ciąg: dla każdego napisu zapisujemy jego długość w bajtach UTF-8 przez VLE, a potem bajty napisu. Cały ciąg kompresujemy jako blok LZ4. Ostatecznie zawartość bloku VARCHAR to `UNCOMPRESSED_LENGTH[VLE]`, a następnie skompresowane bajty LZ4. Format nie narzuca stałego limitu długości pojedynczego napisu.

## Reprezentacja w pamięci

Batch przechowuje kolumny osobno w wektorach (`Vec<i64>`, `Vec<u8>` z wartościami 0/1 lub `Vec<String>`). Metody `cell` i `cell_named` dają dostęp do wartości wiersza. `concat_varchar` tworzy nową kolumnę z konkatenacji dwóch kolumn VARCHAR, wiersz po wierszu.

`FileReader::next_batch` czyta co najwyżej 1024 wiersze naraz. Program aktualizuje metryki po każdej partii, bez gromadzenia całej tabeli w pamięci. Średnia jest liczona z sumy `i128` (dla dokładności), a następnie przedstawiana jako `f64`.
