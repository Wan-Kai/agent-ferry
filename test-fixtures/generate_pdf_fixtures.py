from io import BytesIO
from pathlib import Path

from pypdf import PdfReader, PdfWriter
from reportlab.lib.pagesizes import letter
from reportlab.pdfgen import canvas


ROOT = Path(__file__).resolve().parent


def build_text_pdf() -> bytes:
    """生成包含稳定题录、章节和跨页正文的测试论文。"""
    output = BytesIO()
    document = canvas.Canvas(output, pagesize=letter)
    document.setTitle("A Lightweight PDF Ferry for Research Agents")
    document.setAuthor("Alice Example and Bob Researcher")

    y = 740
    document.setFont("Helvetica-Bold", 18)
    document.drawString(72, y, "A Lightweight PDF Ferry for Research Agents")
    y -= 28
    document.setFont("Helvetica", 11)
    document.drawString(72, y, "Alice Example and Bob Researcher")
    y -= 34
    document.setFont("Helvetica-Bold", 14)
    document.drawString(72, y, "Abstract")
    y -= 20
    document.setFont("Helvetica", 10)
    document.drawString(72, y, "We present a browser-to-agent pipeline that extracts complete PDF text and preserves source metadata.")
    y -= 15
    document.drawString(72, y, "The pipeline reports integrity limitations explicitly instead of silently losing unavailable content.")
    y -= 34
    document.setFont("Helvetica-Bold", 14)
    document.drawString(72, y, "1 Introduction")
    y -= 20
    document.setFont("Helvetica", 10)
    paragraphs = [
        "Research papers opened in a browser often use a built-in PDF viewer rather than a normal HTML document.",
        "A dedicated extractor must fetch the original bytes, validate the source, and preserve all available text.",
        "The extracted Markdown continues through the existing chunked handoff protocol without silent truncation.",
        "Clear errors distinguish download failures, encrypted files, damaged files, and documents without text.",
        "This fixture includes enough prose to exercise completeness thresholds and section recognition reliably.",
    ]
    for _ in range(4):
        for line in paragraphs:
            document.drawString(72, y, line)
            y -= 15
    document.showPage()

    y = 740
    document.setFont("Helvetica-Bold", 14)
    document.drawString(72, y, "2 Method")
    y -= 22
    document.setFont("Helvetica", 10)
    for _ in range(5):
        for line in paragraphs[1:]:
            document.drawString(72, y, line)
            y -= 15
    document.setFont("Helvetica-Bold", 14)
    document.drawString(72, y, "References")
    y -= 22
    document.setFont("Helvetica", 10)
    document.drawString(72, y, "[1] Example Author. Reliable PDF extraction for research agents. 2026.")
    document.save()
    return output.getvalue()


def write_fixtures() -> None:
    text_pdf = build_text_pdf()
    (ROOT / "arxiv-paper.pdf").write_bytes(text_pdf)

    reader = PdfReader(BytesIO(text_pdf))
    writer = PdfWriter()
    writer.append_pages_from_reader(reader)
    writer.add_metadata(reader.metadata or {})
    writer.encrypt("fixture-password")
    with (ROOT / "arxiv-protected.pdf").open("wb") as output:
        writer.write(output)

    blank = canvas.Canvas(str(ROOT / "arxiv-no-text.pdf"), pagesize=letter)
    blank.rect(72, 500, 468, 180, fill=0)
    blank.line(72, 470, 540, 470)
    blank.save()

    (ROOT / "arxiv-corrupt.pdf").write_bytes(b"%PDF-1.7\ncorrupted fixture without xref or objects\n%%EOF\n")


if __name__ == "__main__":
    write_fixtures()
