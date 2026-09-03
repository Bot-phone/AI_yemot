/**
 * AI_yemot - Google Apps Script Backend
 * מערכת AI לעדכון שלוחות בימות המשיח
 */

function doGet(e) {
  return ContentService.createTextOutput(JSON.stringify({
    status: "OK",
    message: "AI_yemot backend is running"
  })).setMimeType(ContentService.MimeType.JSON);
}

function doPost(e) {
  return ContentService.createTextOutput(JSON.stringify({
    status: "OK",
    message: "AI_yemot backend is running"
  })).setMimeType(ContentService.MimeType.JSON);
}
