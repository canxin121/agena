"""Invoke only a selected, local MarkItDown converter; never auto-route input."""
import pathlib
import sys
import zipfile

path = pathlib.Path(sys.argv[1])
extension = sys.argv[2]
try:
    from markitdown import StreamInfo
    from markitdown.converters import DocxConverter, PdfConverter, PptxConverter, XlsxConverter
except ImportError:
    sys.stderr.write("Install markitdown with the docx,pdf,pptx,xlsx extras in AGENA_DOCUMENT_PYTHON.\n")
    sys.exit(2)

converters = {"pdf": PdfConverter, "docx": DocxConverter, "pptx": PptxConverter, "xlsx": XlsxConverter}
if extension not in converters:
    sys.stderr.write("Unsupported local document format.\n")
    sys.exit(2)

try:
    if extension != "pdf":
        with zipfile.ZipFile(path) as archive:
            members = archive.infolist()
            if len(members) > 4096 or sum(item.file_size for item in members) > 64 * 1024 * 1024:
                raise ValueError("Office archive exceeds 4096 entries or 64 MiB expanded content")
            if any(item.flag_bits & 1 for item in members):
                raise ValueError("Encrypted Office archives are not supported")
    with path.open("rb") as stream:
        result = converters[extension]().convert(stream, StreamInfo(extension="." + extension))
    sys.stdout.write(result.markdown)
except Exception as error:
    # Preserve the actual failure class/message without an unbounded traceback.
    sys.stderr.write(f"{type(error).__name__}: {str(error)[:2000]}\n")
    sys.exit(1)
