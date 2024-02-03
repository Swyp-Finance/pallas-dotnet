namespace PallasDotnet.Models;

public enum DatumType {
    NoDatum,
    DatumHash,
    InlineDatum
}

public record Datum(DatumType Type, byte[] Data);

public record TransactionOutput (
    Address Address,
    Value Amount,
    nuint Index,
    Datum? Datum
);