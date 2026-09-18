FROM python:3.11.11-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends libgl1 libgomp1 libglib2.0-0 poppler-utils

RUN useradd -m appuser

WORKDIR /opt/app

COPY requirements.txt /opt/app/
RUN pip install --upgrade pip \
    && pip install --no-cache-dir -r requirements.txt

COPY . /opt/app/
RUN chown -R appuser:appuser /opt/app
USER appuser

CMD ["python3", "/opt/app/main.py"] 
# CMD [ "--host", "0.0.0.0", "--port", "6999", "--workers", "2"]
